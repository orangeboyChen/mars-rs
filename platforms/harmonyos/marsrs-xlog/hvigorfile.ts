// Hvigor's own file for the module: which tasks a `hvigorw assembleHar` runs,
// and nothing else. There is no custom plugin, because there is nothing this
// module's build has to do that `harTasks` does not — which is the point of
// building the natives in `scripts/build_harmony_napi.sh` instead.
import { harTasks } from '@ohos/hvigor-ohos-plugin';

export default {
    system: harTasks,
    plugins: [],
}
