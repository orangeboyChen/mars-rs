package io.github.orangeboychen.marsrs.comm

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Process
import android.os.SystemClock
import android.util.Log
import java.util.HashMap
import java.util.Locale
import java.util.TreeSet

/**
 * The timer the network component `stn` drives its task queue and its reconnect
 * interval with.
 *
 * One [external] is here and it is the only one of the class: [onAlarm], the
 * `Java_io_github_orangeboychen_marsrs_comm_Alarm_onAlarm` symbol `marsrs-jni`
 * exports. It is an *instance* native — JNI is handed `this` — while `start`,
 * `stop` and `resetAlarm` are the statics the C++ project's `Alarm` has too.
 *
 * The C++ project's `Alarm` also keeps a `WakerLock`; the port takes the wake
 * lock itself, on the thread that heard the alarm (`marsrs-jni`'s
 * `START_ALARM_WAKELOCK_MS`), which is where the C++ takes it too and the only
 * place acquiring one works.
 */
class Alarm : BroadcastReceiver() {

    /** One entry of [alarmWaitingSet], ordered — and deduplicated — by its id. */
    private class Waiting(val id: Long, var waittime: Long, val pendingIntent: PendingIntent)

    private external fun onAlarm(id: Long)

    override fun onReceive(context: Context?, intent: Intent?) {
        if (context == null || intent == null) return

        val id = intent.getLongExtra(KEXTRA_ID, 0)
        val pid = intent.getIntExtra(KEXTRA_PID, 0)

        if (id == 0L || pid == 0) return

        // A pid is a number Android hands out again: the `ALARM_ACTION` of a
        // process that was killed is delivered to the process that was given
        // its pid next, which is this one, and an alarm that process set is
        // not one of this one's.
        if (pid != Process.myPid()) {
            Log.w(TAG, String.format(Locale.US, "onReceive id:%d, pid:%d, mypid:%d", id, pid, Process.myPid()))
            return
        }

        val found = synchronized(alarmWaitingSet) {
            val iterator = alarmWaitingSet.iterator()
            var hit = false
            while (iterator.hasNext()) {
                val next = iterator.next()
                Log.i(TAG, String.format(Locale.US, "onReceive id=%d, curId=%d", id, next.id))
                if (next.id == id) {
                    Log.i(
                        TAG,
                        String.format(
                            Locale.US,
                            "onReceive find alarm id:%d, pid:%d, delta miss time:%d",
                            id,
                            pid,
                            SystemClock.elapsedRealtime() - next.waittime
                        )
                    )
                    iterator.remove()
                    requestCodes.remove(id)
                    hit = true
                    break
                }
            }
            if (!hit) {
                Log.e(
                    TAG,
                    String.format(
                        Locale.US,
                        "onReceive not found id:%d, pid:%d, alarm_waiting_set.size:%d",
                        id,
                        pid,
                        alarmWaitingSet.size
                    )
                )
            }
            hit
        }

        if (found) onAlarm(id)
    }

    companion object {

        private const val TAG = "MicroMsg.Alarm"
        private const val KEXTRA_ID = "ID"
        private const val KEXTRA_PID = "PID"

        private val alarmWaitingSet = TreeSet<Waiting>(compareBy { it.id })

        /**
         * The request code the `PendingIntent` of each live alarm goes out
         * under.
         *
         * `PendingIntent.getBroadcast` asks for an `Int` and an id of STN's is
         * a `Long`: two ids that agree in their low 32 bits are one request
         * code, and `FLAG_CANCEL_CURRENT` is then what the second
         * `getBroadcast` does to the first one's `PendingIntent` — it cancels
         * it, and an alarm whose `PendingIntent` was cancelled is an alarm
         * that never fires. So a code of its own is handed out per id instead
         * of read off one, and given back where the alarm leaves
         * [alarmWaitingSet].
         */
        private val requestCodes = HashMap<Long, Int>()

        /** The next request code [requestCodeFor] hands out. */
        private var nextRequestCode = 1

        private var bcAlarm: Alarm? = null

        @JvmStatic
        fun resetAlarm(context: Context) {
            synchronized(alarmWaitingSet) {
                val iterator = alarmWaitingSet.iterator()
                while (iterator.hasNext()) {
                    cancelAlarmMgr(context, iterator.next().pendingIntent)
                }
                alarmWaitingSet.clear()
                requestCodes.clear()
                bcAlarm?.let {
                    context.unregisterReceiver(it)
                    bcAlarm = null
                }
            }
        }

        @JvmStatic
        fun start(id: Long, after: Int, context: Context?): Boolean {
            val curtime = SystemClock.elapsedRealtime()

            if (0 > after) {
                Log.e(TAG, String.format(Locale.US, "id:%d, after:%d", id, after))
                return false
            }

            if (context == null) {
                Log.e(TAG, String.format(Locale.US, "null==context, id:%d, after:%d", id, after))
                return false
            }

            synchronized(alarmWaitingSet) {
                if (bcAlarm == null) {
                    val alarm = Alarm()
                    bcAlarm = alarm
                    context.registerReceiver(alarm, IntentFilter("ALARM_ACTION(${Process.myPid()})"))
                }

                val iterator = alarmWaitingSet.iterator()
                while (iterator.hasNext()) {
                    if (iterator.next().id == id) {
                        Log.e(TAG, String.format(Locale.US, "id exist=%d", id))
                        return false
                    }
                }

                val waittime = if (after >= 0) curtime + after else curtime

                val pendingIntent = setAlarmMgr(id, waittime, context) ?: return false

                alarmWaitingSet.add(Waiting(id, waittime, pendingIntent))
            }
            return true
        }

        @JvmStatic
        fun stop(id: Long, context: Context?): Boolean {
            if (context == null) {
                Log.e(TAG, "context==null")
                return false
            }

            synchronized(alarmWaitingSet) {
                // Nothing was ever started, so there is no receiver to stop and
                // no alarm to cancel. Registering one here to have something to
                // unregister would put a receiver on the air with an empty
                // `IntentFilter` — which is every broadcast Android sends, and
                // not the `ALARM_ACTION` [start] asks for — and nothing but
                // [resetAlarm] would ever take it off again.
                if (bcAlarm == null) {
                    return false
                }

                val iterator = alarmWaitingSet.iterator()
                while (iterator.hasNext()) {
                    val next = iterator.next()
                    if (next.id == id) {
                        cancelAlarmMgr(context, next.pendingIntent)
                        iterator.remove()
                        requestCodes.remove(id)
                        return true
                    }
                }
            }

            return false
        }

        /**
         * The request code the alarm of `id` goes out under: the one it already
         * has while it is live, and a fresh one otherwise.
         *
         * [requestCodes] and [alarmWaitingSet] are read and written under the
         * same lock, so an id that is in one is in the other.
         */
        private fun requestCodeFor(id: Long): Int {
            val given = requestCodes[id]
            if (given != null) {
                return given
            }
            val fresh = nextRequestCode
            nextRequestCode += 1
            requestCodes[id] = fresh
            return fresh
        }

        private fun setAlarmMgr(id: Long, time: Long, context: Context): PendingIntent? {
            val am = context.getSystemService(Context.ALARM_SERVICE) as? AlarmManager
            if (am == null) {
                Log.e(TAG, "am == null")
                return null
            }

            val intent = Intent()
            intent.setAction("ALARM_ACTION(${Process.myPid()})")
            intent.putExtra(KEXTRA_ID, id)
            intent.putExtra(KEXTRA_PID, Process.myPid())

            val flags = if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M) {
                PendingIntent.FLAG_CANCEL_CURRENT
            } else {
                PendingIntent.FLAG_CANCEL_CURRENT or PendingIntent.FLAG_IMMUTABLE
            }
            val pendingIntent = PendingIntent.getBroadcast(context, requestCodeFor(id), intent, flags)

            // `set` is the one Android batches from KITKAT on, and an alarm it
            // batches is one it fires inside a window of its own choosing —
            // which is a task queue that wakes when Android wakes it and not
            // when STN asked.
            @Suppress("DEPRECATION")
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.KITKAT) {
                am.set(AlarmManager.ELAPSED_REALTIME_WAKEUP, time, pendingIntent)
            } else {
                am.setExact(AlarmManager.ELAPSED_REALTIME_WAKEUP, time, pendingIntent)
            }

            return pendingIntent
        }

        private fun cancelAlarmMgr(context: Context, pendingIntent: PendingIntent?): Boolean {
            val am = context.getSystemService(Context.ALARM_SERVICE) as? AlarmManager
            if (am == null) {
                Log.e(TAG, "am == null")
                return false
            }
            if (pendingIntent == null) {
                Log.e(TAG, "pendingIntent == null")
                return false
            }

            am.cancel(pendingIntent)
            pendingIntent.cancel()
            return true
        }
    }
}
