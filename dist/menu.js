// One positioned popup menu for row actions, recent files and saved filters.
// Items are [label, action, title?, {checked}?]; null draws a separator, an
// item without an action is a note, and `checked` marks one of a set of
// choices (focused first). Escape restores focus to the opener; Tab, outside
// presses, window blur/resize and list scrolling close it.
export function createPopupMenu({menu, document, window, scrollers=[]}) {
  let opener=null;
  function close(restore=false){
    if(menu.hidden)return;
    menu.hidden=true;menu.replaceChildren();
    if(restore)opener?.focus?.({preventScroll:true});opener=null;
  }
  function show(items,x,y,label){
    opener=document.activeElement;menu.setAttribute('aria-label',label);
    menu.replaceChildren(...items.map(item=>{
      if(!item){const line=document.createElement('hr');line.setAttribute('role','separator');return line;}
      if(!item[1]){const note=document.createElement('p');note.className='menu-note';note.textContent=item[0];return note;}
      const button=document.createElement('button');button.type='button';button.setAttribute('role','menuitem');button.tabIndex=-1;button.textContent=item[0];if(item[2])button.title=item[2];
      if(item[3]&&'checked' in item[3]){button.setAttribute('role','menuitemradio');button.setAttribute('aria-checked',String(!!item[3].checked));}
      button.addEventListener('click',()=>{close(true);item[1]();});return button;
    }));
    menu.hidden=false;
    // Keep the menu inside the window.
    const box=menu.getBoundingClientRect?.(),width=window.innerWidth||0,height=window.innerHeight||0;
    menu.style.left=`${Math.max(4,box&&width?Math.min(x,width-box.width-4):x)}px`;
    menu.style.top=`${Math.max(4,box&&height?Math.min(y,height-box.height-4):y)}px`;
    // A menu of notes only takes focus itself, so Escape still closes it.
    const first=menu.querySelector?.('[aria-checked=true]')??menu.querySelector?.('[role^=menuitem]');
    if(first)first.focus();else{menu.tabIndex=-1;menu.focus?.();}
  }
  menu.addEventListener('keydown',event=>{
    const items=[...(menu.querySelectorAll?.('[role^=menuitem]')??[])],at=items.indexOf(document.activeElement);
    if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close(true);}
    else if(event.key==='Tab')close(false);
    else if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)&&items.length){
      event.preventDefault();
      const next=event.key==='Home'?0:event.key==='End'?items.length-1:(at+(event.key==='ArrowDown'?1:-1)+items.length)%items.length;
      items[next].focus();
    }
  });
  document.addEventListener('pointerdown',event=>{if(!menu.hidden&&!menu.contains(event.target))close(false);},true);
  window.addEventListener('blur',()=>close(false));window.addEventListener('resize',()=>close(false));
  for(const scroller of scrollers)scroller.addEventListener('scroll',()=>close(false));
  return {show,close};
}
