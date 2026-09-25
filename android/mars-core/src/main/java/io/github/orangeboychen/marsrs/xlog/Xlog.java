package io.github.orangeboychen.marsrs.xlog;

/**
 * The Java face of {@code libmarsxlog.so} (crate {@code mars-jni}).
 *
 * Every {@code native} here is one of the {@code Java_io_github_orangeboychen_marsrs_xlog_Xlog_*}
 * symbols of that crate, and the field names of {@link XLogConfig} are the ones
 * its {@code config_from_java} reads, so the two must be changed together.
 *
 * The library is loaded by {@link #open}: it is {@code marsxlog}, the
 * {@code crate-name} of {@code mars-jni}.
 */
public class Xlog implements Log.LogImp {

    public static final int LEVEL_ALL = 0;
    public static final int LEVEL_VERBOSE = 0;
    public static final int LEVEL_DEBUG = 1;
    public static final int LEVEL_INFO = 2;
    public static final int LEVEL_WARNING = 3;
    public static final int LEVEL_ERROR = 4;
    public static final int LEVEL_FATAL = 5;
    public static final int LEVEL_NONE = 6;

    public static final int AppednerModeAsync = 0;
    public static final int AppednerModeSync = 1;

    public static final int ZLIB_MODE = 0;
    public static final int ZSTD_MODE = 1;

    /** {@code XLoggerInfo}: what {@link #logWrite} reads out of its argument. */
    public static class XLoggerInfo {
        public int level;
        public String tag;
        public String filename;
        public String funcname;
        public int line;
        public long pid;
        public long tid;
        public long maintid;
    }

    /** {@code XLogConfig} — the fields {@code config_from_java} reads by name. */
    public static class XLogConfig {
        public int level = LEVEL_INFO;
        public int mode = AppednerModeAsync;
        public String logdir;
        public String nameprefix;
        public String pubkey = "";
        public int compressmode = ZLIB_MODE;
        public int compresslevel = 0;
        public String cachedir;
        public int cachedays = 0;
    }

    /**
     * Loads {@code libmarsxlog.so} and opens the process-wide appender.
     *
     * @param isLoadLib whether to load the library here; pass {@code false} when
     *                  the app has loaded it already
     */
    public static void open(boolean isLoadLib, int level, int mode, String cacheDir, String logDir,
                            String nameprefix, String pubkey) {
        if (isLoadLib) {
            System.loadLibrary("marsxlog");
        }

        XLogConfig logConfig = new XLogConfig();
        logConfig.level = level;
        logConfig.mode = mode;
        logConfig.logdir = logDir;
        logConfig.nameprefix = nameprefix;
        logConfig.pubkey = pubkey;
        logConfig.compressmode = ZLIB_MODE;
        logConfig.compresslevel = 0;
        logConfig.cachedir = cacheDir;
        logConfig.cachedays = 0;
        appenderOpen(logConfig);
    }

    public static void logWrite2(int level, String tag, String filename, String funcname, int line,
                                 int pid, long tid, long maintid, String log) {
        logWrite2(0, level, tag, filename, funcname, line, pid, tid, maintid, log);
    }

    public static native void logWrite(XLoggerInfo logInfo, String log);

    public static native void logWrite2(long logInstancePtr, int level, String tag, String filename,
                                        String funcname, int line, int pid, long tid, long maintid,
                                        String log);

    public native int getLogLevel(long logInstancePtr);

    public native void setLogLevel(long logInstancePtr, int level);

    public native void setAppenderMode(long logInstancePtr, int mode);

    public native long getXlogInstance(String nameprefix);

    public native void releaseXlogInstance(String nameprefix);

    public native long newXlogInstance(XLogConfig logConfig);

    /** Whether the console prints the log too. */
    public native void setConsoleLogOpen(long logInstancePtr, boolean isOpen);

    public native void appenderClose();

    public native void appenderFlush(long logInstancePtr, boolean isSync);

    public native void setMaxFileSize(long logInstancePtr, long size);

    public native void setMaxAliveTime(long logInstancePtr, long seconds);

    private static native void appenderOpen(XLogConfig logConfig);

    // #################### Log.LogImp ####################
    //
    // `Log` is the facade the C++ project's `Log.java` is: `Log.d(tag, msg)`
    // and friends, over whichever `LogImp` the app handed to
    // `Log.setLogImp` — `new Xlog()`, here. Everything below is a straight
    // call of a native above.

    @Override
    public void logV(long logInstancePtr, String tag, String filename, String funcname, int line, int pid, long tid, long maintid, String log) {
        logWrite2(logInstancePtr, LEVEL_VERBOSE, tag, filename, funcname, line, pid, tid, maintid, log);
    }

    @Override
    public void logD(long logInstancePtr, String tag, String filename, String funcname, int line, int pid, long tid, long maintid, String log) {
        logWrite2(logInstancePtr, LEVEL_DEBUG, tag, filename, funcname, line, pid, tid, maintid, log);
    }

    @Override
    public void logI(long logInstancePtr, String tag, String filename, String funcname, int line, int pid, long tid, long maintid, String log) {
        logWrite2(logInstancePtr, LEVEL_INFO, tag, filename, funcname, line, pid, tid, maintid, log);
    }

    @Override
    public void logW(long logInstancePtr, String tag, String filename, String funcname, int line, int pid, long tid, long maintid, String log) {
        logWrite2(logInstancePtr, LEVEL_WARNING, tag, filename, funcname, line, pid, tid, maintid, log);
    }

    @Override
    public void logE(long logInstancePtr, String tag, String filename, String funcname, int line, int pid, long tid, long maintid, String log) {
        logWrite2(logInstancePtr, LEVEL_ERROR, tag, filename, funcname, line, pid, tid, maintid, log);
    }

    @Override
    public void logF(long logInstancePtr, String tag, String filename, String funcname, int line, int pid, long tid, long maintid, String log) {
        logWrite2(logInstancePtr, LEVEL_FATAL, tag, filename, funcname, line, pid, tid, maintid, log);
    }

    @Override
    public long openLogInstance(int level, int mode, String cacheDir, String logDir, String nameprefix, int cacheDays) {
        XLogConfig logConfig = new XLogConfig();
        logConfig.level = level;
        logConfig.mode = mode;
        logConfig.logdir = logDir;
        logConfig.nameprefix = nameprefix;
        logConfig.cachedir = cacheDir;
        logConfig.cachedays = cacheDays;
        return newXlogInstance(logConfig);
    }

    @Override
    public void appenderOpen(int level, int mode, String cacheDir, String logDir, String nameprefix, int cacheDays) {
        XLogConfig logConfig = new XLogConfig();
        logConfig.level = level;
        logConfig.mode = mode;
        logConfig.logdir = logDir;
        logConfig.nameprefix = nameprefix;
        logConfig.cachedir = cacheDir;
        logConfig.cachedays = cacheDays;
        appenderOpen(logConfig);
    }
}
