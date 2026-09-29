// Hvigor's own file for the project: which tasks a `hvigorw assembleHap` runs,
// and nothing else. `appTasks` is the built-in set for a project with an app in
// it — the module below has `hapTasks` — and a demo needs no plugin besides.
import { appTasks } from '@ohos/hvigor-ohos-plugin';

export default {
    system: appTasks,  /* Built-in plugin of Hvigor. It cannot be modified. */
    plugins: [],       /* Custom plugin to extend the functionality of Hvigor. */
}
