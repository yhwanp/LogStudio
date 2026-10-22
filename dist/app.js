import {mountLargeLogView} from './large-view.js';
import {desktopTransport} from './desktop.js';
import {mountZoom} from './zoom.js';
import {mountShortcutHelp} from './shortcuts.js';
import {mountTips} from './tips.js';
const tauri=window.__TAURI__;
const $=id=>document.getElementById(id);
const help=mountShortcutHelp();
// The tour runs on the open file, or on the sample log when none is open.
const tips=mountTips({closeHelp:()=>help?.close(),needsFile:()=>!$('emptyState').hidden,openSample:()=>$('loadDemo').click()});
if(!tauri){$('largeLogPanel').hidden=false;$('largeError').textContent='LogStudio 데스크톱 앱을 실행해 주세요.';$('largeError').hidden=false;$('errorBar').hidden=false;}
else{
 mountZoom({setZoom:level=>tauri.webview.getCurrentWebview().setZoom(level),indicator:$('zoomIndicator')});
 mountLargeLogView(tauri,{transport:desktopTransport(tauri),tips});
}
