// Hvigor's own file for the module: `hapTasks` and not `harTasks`, because what
// this module builds is a HAP a device installs and not a library an app
// compiles into itself.
import { hapTasks } from '@ohos/hvigor-ohos-plugin';

export default {
    system: hapTasks,  /* Built-in plugin of Hvigor. It cannot be modified. */
    plugins: [],       /* Custom plugin to extend the functionality of Hvigor. */
}
