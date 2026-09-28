// What `import { Xlog } from 'marsrs-harmonyos-xlog'` answers: the whole surface
// of the package, and nothing of the NAPI module behind it — `libmarsrs_xlog.so`
// is reached through `Xlog` and not by an app of its own, so the one thing the
// entry point exports that is not a type is the class that owns the appender.

export { LogLevel } from './src/main/ets/xlog/LogLevel';
export { AppenderMode } from './src/main/ets/xlog/AppenderMode';
export { CompressMode } from './src/main/ets/xlog/CompressMode';
export { XlogConfig } from './src/main/ets/xlog/XlogConfig';
export { Xlog } from './src/main/ets/xlog/Xlog';
