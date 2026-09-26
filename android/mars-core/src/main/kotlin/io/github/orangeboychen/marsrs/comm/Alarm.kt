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
import io.github.orangeboychen.marsrs.xlog.Log
import java.util.TreeSet

/**
 * The timer the network component `stn` drives its task queue and its reconnect
 * interval with.
 *
 * One [external] is here and it is the only one of the class: [onAlarm], the
 * `Java_io_github_orangeboychen_marsrs_comm_Alarm_onAlarm` symbol `mars-jni`
 * exports. It is an *instance* native — JNI is handed `this` — while `start`,
 * `stop` and `resetAlarm` are the statics the C++ project's `Alarm` has too.
 *
 * The C++ project's `Alarm` also keeps a `WakerLock`; the port takes the wake
 * lock itself, on the thread that heard the alarm (`mars-jni`'s
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

        if (pid != Process.myPid()) {
            Log.w(TAG, "onReceive id:%d, pid:%d, mypid:%d", id, pid, Process.myPid())
            return
        }

        val found = synchronized(alarmWaitingSet) {
            val iterator = alarmWaitingSet.iterator()
            var hit = false
            while (iterator.hasNext()) {
                val next = iterator.next()
                Log.i(TAG, "onReceive id=%d, curId=%d", id, next.id)
                if (next.id == id) {
                    Log.i(
                        TAG,
                        "onReceive find alarm id:%d, pid:%d, delta miss time:%d",
                        id,
                        pid,
                        SystemClock.elapsedRealtime() - next.waittime
                    )
                    iterator.remove()
                    hit = true
                    break
                }
            }
            if (!hit) {
                Log.e(
                    TAG,
                    "onReceive not found id:%d, pid:%d, alarm_waiting_set.size:%d",
                    id,
                    pid,
                    alarmWaitingSet.size
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

        private var bcAlarm: Alarm? = null

        @JvmStatic
        fun resetAlarm(context: Context) {
            synchronized(alarmWaitingSet) {
                val iterator = alarmWaitingSet.iterator()
                while (iterator.hasNext()) {
                    cancelAlarmMgr(context, iterator.next().pendingIntent)
                }
                alarmWaitingSet.clear()
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
                Log.e(TAG, "id:%d, after:%d", id, after)
                return false
            }

            if (context == null) {
                Log.e(TAG, "null==context, id:%d, after:%d", id, after)
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
                        Log.e(TAG, "id exist=%d", id)
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
                if (bcAlarm == null) {
                    val alarm = Alarm()
                    bcAlarm = alarm
                    context.registerReceiver(alarm, IntentFilter())
                    Log.i(TAG, "stop new Alarm")
                }

                val iterator = alarmWaitingSet.iterator()
                while (iterator.hasNext()) {
                    val next = iterator.next()
                    if (next.id == id) {
                        cancelAlarmMgr(context, next.pendingIntent)
                        iterator.remove()
                        return true
                    }
                }
            }

            return false
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
            val pendingIntent = PendingIntent.getBroadcast(context, id.toInt(), intent, flags)

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
