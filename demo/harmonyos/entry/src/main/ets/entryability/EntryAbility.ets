// The ability of the demo, and the whole of the lifecycle an app needs: open the
// appender when the app starts, load the page, and close the appender when the
// app goes.
//
// The stage model gives an app an `onDestroy` and not a "the process is being
// killed" signal, so this is the last moment an app is told anything at all —
// and with an async appender, closing here is what keeps the last records from
// going with the process.

import { UIAbility } from '@kit.AbilityKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';
import { closeLogStore, openLogStore } from '../model/LogStore';

/** The tag and the domain DevEco's own HiLog calls carry. */
const DOMAIN: number = 0x0000;

const TAG: string = 'MarsRSDemo';

export default class EntryAbility extends UIAbility {
  onCreate(): void {
    // The appender is opened here and not in the page, because `filesDir` — the
    // one directory an app may write without a permission — is a thing the
    // ability has and a page does not.
    openLogStore(this.context);
  }

  onDestroy(): void {
    closeLogStore();
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    // `pages/Index`, which is what `main_pages.json` lists.
    windowStage.loadContent('pages/Index', (error) => {
      if (error.code) {
        hilog.error(DOMAIN, TAG, 'the page was not loaded: %{public}s', JSON.stringify(error));
        return;
      }
      hilog.info(DOMAIN, TAG, 'the page was loaded');
    });
  }

  onWindowStageDestroy(): void {
    hilog.info(DOMAIN, TAG, 'the window stage went');
  }

  onForeground(): void {
    hilog.info(DOMAIN, TAG, 'the app came back');
  }

  onBackground(): void {
    hilog.info(DOMAIN, TAG, 'the app went to the background');
  }
}
