import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {setImmediate as nextTurn} from 'node:timers/promises';
import {replacesLog, describePaste, describeClosing, mountPasteConfirm, createPasteFlow} from '../dist/paste-confirm.js';
import {describeLogError} from '../dist/describe.js';

async function settle(){for(let i=0;i<10;i++)await nextTurn();}
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};

test('a paste replaces a shown, loading or opening log, not the empty view',()=>{
  assert.equal(replacesLog({datasetId:3,closed:false,status:'ready'}),true);
  assert.equal(replacesLog({datasetId:3,closed:false,status:'indexing',textComplete:false}),true);
  for(const state of [{datasetId:null,closed:false,status:'empty'},{datasetId:3,closed:true,status:'ready'},{datasetId:3,closed:false,status:'empty'},{},null])assert.equal(replacesLog(state),false,JSON.stringify(state));
  assert.equal(replacesLog({datasetId:null,status:'connecting'},true),true,'a log being opened');
});

test('the dialog says what opens and, only when true, that the work is kept',()=>{
  assert.equal(describePaste({kind:'text',lines:1234,size:56*1024}),'로그 텍스트 1,234줄 · 56.0 KiB');
  assert.equal(describePaste({kind:'file',name:'device main.log'}),'파일 device main.log');
  for(const name of [undefined,'',7])assert.equal(describePaste({kind:'file',name}),'복사한 파일');
  assert.equal(describePaste({kind:'file',name:'a'.repeat(400)}).length,'파일 '.length+255);
  assert.match(describeClosing(true),/닫힙니다.*저장되어 같은 파일을 다시 열면 복원/);
  assert.equal(describeClosing(false),'열려 있는 로그는 닫히며 다시 열 수 없습니다.');
  assert.equal(describeClosing(null),'열려 있는 로그는 닫힙니다.');
});

class Element {
  constructor(document,tag='div'){this.ownerDocument=document;this.tagName=tag.toUpperCase();this.listeners=new Map();this.isConnected=true;this.hidden=false;this.textContent='';}
  addEventListener(name,fn){(this.listeners.get(name)??this.listeners.set(name,[]).get(name)).push(fn);}
  fire(name,event={}){let prevented=false,stopped=false;const full={target:this,preventDefault(){prevented=true;},stopPropagation(){stopped=true;},...event};for(const fn of this.listeners.get(name)??[])fn(full);return {prevented,stopped};}
  focus(){this.ownerDocument.activeElement=this;}
}
// A native modal dialog: close() fires `close`, here at once.
class Dialog extends Element {
  constructor(document){super(document,'dialog');this.open=false;}
  showModal(){this.open=true;}
  close(){if(!this.open)return;this.open=false;this.fire('close');}
  contains(node){return Object.values(this.parts).includes(node);}
}
function rig(){
  const document={activeElement:null,body:null};document.body=new Element(document,'body');
  const dialog=new Dialog(document),parts={pasteForm:new Element(document,'form'),pasteWhat:new Element(document,'p'),pasteNote:new Element(document,'p'),pasteOpen:new Element(document,'button'),pasteCancel:new Element(document,'button')};
  dialog.parts=parts;const elements={pasteDialog:dialog,...parts};
  document.getElementById=id=>elements[id]??null;
  return {document,dialog,...parts,confirm:mountPasteConfirm({document})};
}

test('열기 is focused and answers open; 취소, Escape and the backdrop answer cancel; focus returns',async()=>{
  const r=rig(),row=new Element(r.document);
  row.focus();let answer=r.confirm.ask({what:'로그 텍스트 2줄 · 10 B',note:'열려 있는 로그는 닫힙니다.'});
  assert.equal(r.dialog.open,true);assert.equal(r.pasteWhat.textContent,'로그 텍스트 2줄 · 10 B');assert.equal(r.pasteNote.textContent,'열려 있는 로그는 닫힙니다.');
  assert.equal(r.document.activeElement,r.pasteOpen);
  assert.equal(r.pasteForm.fire('submit').prevented,true);assert.equal(await answer,'open');
  assert.equal(r.dialog.open,false);assert.equal(r.document.activeElement,row);
  for(const [close,label] of [[()=>r.pasteCancel.fire('click'),'취소'],[()=>assert.deepEqual(r.dialog.fire('keydown',{key:'Escape'}),{prevented:true,stopped:true}),'Escape'],[()=>assert.equal(r.dialog.fire('cancel').prevented,true),'native cancel'],[()=>r.dialog.fire('click'),'backdrop']]){
    row.focus();answer=r.confirm.ask({what:'x',note:'y'});
    r.dialog.fire('click',{target:r.pasteWhat});assert.equal(r.dialog.open,true,'a press inside the form keeps it');
    r.dialog.fire('keydown',{key:'Enter'});assert.equal(r.dialog.open,true);
    close();assert.equal(await answer,'cancel',label);assert.equal(r.dialog.open,false);assert.equal(r.document.activeElement,row,label);
  }
  // A removed opener is not focused; the body is never focused.
  row.focus();answer=r.confirm.ask({what:'x',note:'y'});row.isConnected=false;r.confirm.dismiss();
  assert.equal(await answer,'cancel');assert.equal(r.document.activeElement,r.pasteOpen);
  r.document.activeElement=r.document.body;answer=r.confirm.ask({what:'x',note:'y'});r.dialog.close();
  assert.equal(await answer,'cancel','closed any other way');assert.equal(r.document.activeElement,r.pasteOpen);
});

test('a newer paste replaces the offer in the open dialog and keeps the first opener',async()=>{
  const r=rig(),row=new Element(r.document);row.focus();
  const first=r.confirm.ask({what:'first',note:'n'});
  const second=r.confirm.ask({what:'second',note:'n'});
  assert.equal(await first,'replaced');assert.equal(r.dialog.open,true);assert.equal(r.pasteWhat.textContent,'second');
  r.pasteForm.fire('submit');assert.equal(await second,'open');assert.equal(r.document.activeElement,row);
  r.confirm.dismiss();assert.equal(r.confirm.isOpen,false);
});

// The flow with a scripted dialog and native side.
function flow({replaces=true,inField=false,hold=async()=>{}}={}){
  const calls=[],opened=[],errors=[],asks=[];let gone=false,shown=replaces,field=inField;
  const dialog={ask(offer){const d=deferred();asks.forEach(a=>a.pending&&(a.pending=false,a.resolve('replaced')));asks.push({...offer,pending:true,resolve:d.resolve});return d.promise;}};
  const answer=result=>{const a=asks.findLast(a=>a.pending);a.pending=false;a.resolve(result);};
  const invoke=async(command,args)=>{calls.push([command,args.grant,args.action]);if(command==='hold_file_paste')return hold(args);return {};};
  const pastes=createPasteFlow({invoke,dialog,replaces:()=>shown,closing:()=>'closing',openText:item=>opened.push(item.text),report:error=>errors.push(String(error)),isGone:()=>gone,inField:()=>field});
  return {pastes,calls,opened,errors,asks,answer,gone:()=>{gone=true;},show:value=>{shown=value;},field:value=>{field=value;}};
}

test('text opens at once with nothing open, and only on 열기 over an open log',async()=>{
  const f=flow({replaces:false});
  await f.pastes.text({kind:'text',text:'a\nb\n',size:4,lines:2});assert.deepEqual(f.opened,['a\nb\n']);assert.equal(f.asks.length,0);
  f.show(true);
  let pending=f.pastes.text({kind:'text',text:'c\n',size:2,lines:1});await settle();
  assert.deepEqual(f.asks.map(a=>[a.what,a.note]),[['로그 텍스트 1줄 · 2 B','closing']]);
  f.answer('cancel');await pending;assert.deepEqual(f.opened,['a\nb\n']);
  pending=f.pastes.text({kind:'text',text:'d\n',size:2,lines:1});await settle();
  const later=f.pastes.text({kind:'text',text:'e\n',size:2,lines:1});await settle();
  f.answer('open');await pending;await later;assert.deepEqual(f.opened,['a\nb\n','e\n'],'the replaced offer never opens');
  pending=f.pastes.text({kind:'text',text:'f\n',size:2,lines:1});await settle();f.gone();f.answer('open');await pending;
  assert.deepEqual(f.opened,['a\nb\n','e\n'],'a page going away opens nothing');
  assert.deepEqual(f.calls,[]);
});

test('a file grant is held while asking, opened on 열기, and dropped without pasting otherwise',async()=>{
  const f=flow();
  let pending=f.pastes.file({grant:'g1',name:'device.log'});await settle();
  assert.equal(f.asks[0].what,'파일 device.log');f.answer('open');await pending;
  assert.deepEqual(f.calls,[['hold_file_paste','g1',undefined],['finish_file_paste','g1','open']]);
  pending=f.pastes.file({grant:'g2'});await settle();f.answer('cancel');await pending;await settle();
  assert.deepEqual(f.calls.slice(2),[['hold_file_paste','g2',undefined],['finish_file_paste','g2','discard']]);
  // A text paste replacing the offer drops the file's grant.
  pending=f.pastes.file({grant:'g3'});await settle();
  const text=f.pastes.text({kind:'text',text:'x\n',size:2,lines:1});await settle();await pending;await settle();
  assert.deepEqual(f.calls.slice(4),[['hold_file_paste','g3',undefined],['finish_file_paste','g3','discard']]);
  f.answer('open');await text;assert.deepEqual(f.opened,['x\n']);
  // A text field, or a page going away, keeps the standard Paste and never asks.
  f.field(true);await f.pastes.file({grant:'g4'});f.field(false);
  f.show(false);await f.pastes.file({grant:'g5'});
  f.gone();await f.pastes.file({grant:'g6'});
  assert.deepEqual(f.calls.slice(6),[['finish_file_paste','g4','paste'],['finish_file_paste','g5','open'],['finish_file_paste','g6','paste']]);
  assert.equal(f.asks.length,4);
  for(const payload of [null,{},{grant:7}])await f.pastes.file(payload);
  assert.equal(f.calls.length,9,'a payload without a grant is ignored');
});

test('a hold that fails reports once and drops the grant; a newer paste during the hold drops the older grant',async()=>{
  let f=flow({hold:async()=>{throw 'Paste grant expired; paste again';}});
  await f.pastes.file({grant:'late'});await settle();
  assert.deepEqual(f.errors,['Paste grant expired; paste again']);assert.equal(f.asks.length,0);
  assert.deepEqual(f.calls,[['hold_file_paste','late',undefined],['finish_file_paste','late','discard']]);
  const holds=[];f=flow({hold:()=>{const d=deferred();holds.push(d);return d.promise;}});
  const older=f.pastes.file({grant:'older'});await settle();
  const newer=f.pastes.file({grant:'newer'});await settle();
  holds[1].resolve();await settle();holds[0].resolve();await older;await settle();
  assert.deepEqual(f.asks.map(a=>a.what),['복사한 파일']);
  assert.deepEqual(f.calls.filter(c=>c[0]==='finish_file_paste'),[['finish_file_paste','older','discard']]);
  f.answer('open');await newer;
  assert.deepEqual(f.calls.at(-1),['finish_file_paste','newer','open']);
  // A failing hold of a superseded paste stays quiet.
  f=flow({hold:()=>{const d=deferred();holds.push(d);return d.promise;}});
  const stale=f.pastes.file({grant:'stale'});await settle();
  const text=f.pastes.text({kind:'text',text:'t\n',size:2,lines:1});await settle();
  holds.at(-1).reject('Paste grant is unavailable');await stale;await settle();
  assert.deepEqual(f.errors,[]);assert.deepEqual(f.calls.at(-1),['finish_file_paste','stale','discard']);
  f.answer('cancel');await text;
});

test('the page has one paste dialog with 취소 before the primary 열기',async()=>{
  const html=await readFile(new URL('../dist/index.html',import.meta.url),'utf8');
  const dialog=html.slice(html.indexOf('<dialog id="pasteDialog"'),html.indexOf('</dialog>',html.indexOf('<dialog id="pasteDialog"')));
  assert.match(dialog,/aria-labelledby="pasteTitle"/);assert.match(dialog,/<h2 id="pasteTitle">붙여넣은 내용으로 로그를 바꿀까요\?<\/h2>/);
  assert.match(dialog,/<button id="pasteCancel" class="button" type="button">취소<\/button><button id="pasteOpen" class="button primary" type="submit">열기<\/button>/);
});

test('paste and drop errors read in Korean, and no files is not the 64-file limit',()=>{
  for(const error of ['Paste grant expired; paste again','Paste grant is unavailable','Paste requires the main window','Select at least one log file','Select 1 to 64 log files']){
    const text=describeLogError(error);assert.match(text,/[가-힣]/,error);assert.ok(!text.includes(error),error);
  }
  assert.doesNotMatch(describeLogError('Select at least one log file'),/64/);
  assert.match(describeLogError('Select 1 to 64 log files'),/64개/);
});
