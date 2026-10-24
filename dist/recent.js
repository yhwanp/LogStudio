// Recent files are native records; the page only names them by opaque ID.
import {textSpan} from './dom.js';

// `guard` asks about a capture's unsaved rows before a recent file replaces
// its log, and `keep` keeps the capture's file when that open fails.
export function mountRecentFiles({ui, document, native, menu, isGone, onError, guard = async () => 'none', keep = () => {}, shown = () => null}) {
  let recent=[];
  // Files opened together reopen together; the tooltip lists them in order.
  const title=item=>Array.isArray(item.files)&&item.files.length>1?`${item.folder}\n${item.files.map((name,i)=>`${i+1}. ${name}`).join('\n')}`:`${item.folder}/${item.name}`;
  function render(list){
    recent=Array.isArray(list)?list.filter(item=>Number.isSafeInteger(item?.id)&&typeof item.name==='string').slice(0,10):[];
    ui.recentFiles.hidden=!recent.length;ui.recentButton.hidden=!recent.length;
    ui.recentList.replaceChildren(...recent.map(item=>{
      const row=document.createElement('li'),open=document.createElement('button'),remove=document.createElement('button');
      open.type='button';open.className='recent-open';open.title=title(item);
      const name=document.createElement('strong');name.textContent=item.name;open.append(name,textSpan(document,item.folder));
      open.addEventListener('click',()=>void openRecent(item.id));
      remove.type='button';remove.className='recent-remove';remove.textContent='×';remove.title='목록에서 제거';remove.setAttribute('aria-label',`${item.name} 목록에서 제거`);
      remove.addEventListener('click',()=>{void native.core.invoke('forget_recent',{id:item.id}).then(render,()=>{});});
      row.append(open,remove);return row;
    }));
  }
  async function openRecent(id){
    if(isGone()||await guard()==='cancel')return;
    const was=shown();
    void native.core.invoke('open_recent',{id}).catch(error=>{keep(was);if(!isGone())onError(String(error).slice(0,500));});
  }
  const forgetAll=()=>{void native.core.invoke('forget_recent',{id:null}).then(render,()=>{});};
  ui.clearRecent.addEventListener('click',forgetAll);
  ui.recentButton.addEventListener('click',()=>{
    const box=ui.recentButton.getBoundingClientRect?.();
    menu.show(recent.map(item=>[item.name,()=>void openRecent(item.id),title(item)]).concat([null,['목록 지우기',forgetAll]]),box?box.left:0,box?box.bottom+4:0,'최근 파일');
  });
  void native.core.invoke('recent_files').then(render,()=>{});
  void native.event.listen('recent-changed',({payload})=>render(payload)).catch(()=>{});
}
