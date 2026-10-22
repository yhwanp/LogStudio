import test from 'node:test';
import assert from 'node:assert/strict';
import {inspectPaste,handleLogPaste,FILE_PASTE_FALLBACK} from '../dist/clipboard.js';
const log='06-18 19:24:24.983  2983 4241 D SyntheticExample: clipboard test';
const event=(data,target={})=>({target,clipboardData:{files:[],items:[],getData:()=>'',...data},prevented:false,preventDefault(){this.prevented=true;}});
test('file list opens one File through the shared callback and prevents default',()=>{
  const file=new File([log],'synthetic-paste.log',{type:'text/plain'}),e=event({files:[file]});let opened;
  assert.ok(handleLogPaste(e,{openFile:f=>opened=f,openText:()=>assert.fail(),onError:()=>assert.fail()}));assert.equal(opened,file);assert.equal(e.prevented,true);
});
test('file items fallback, null items, and multiple files are handled explicitly',()=>{
  const file=new File([log],'synthetic.log');assert.equal(inspectPaste(event({items:[{kind:'file',getAsFile:()=>file}]})).file,file);
  assert.equal(inspectPaste(event({items:[{kind:'file',getAsFile:()=>null}]})).message,FILE_PASTE_FALLBACK);
  assert.equal(inspectPaste(event({files:[file,file]})).kind,'error');
});
test('input, textarea and contenteditable preserve native paste without reading clipboard',()=>{
  for(const target of [{closest:()=>({})},{isContentEditable:true}]){let read=false;const e=event({getData:()=>{read=true;return log;}},target);assert.equal(handleLogPaste(e,{openFile:()=>assert.fail(),openText:()=>assert.fail(),onError:()=>assert.fail()}),false);assert.equal(e.prevented,false);assert.equal(read,false);}
});
test('Windows, UNC, file URI, POSIX paths and filename-only references are not read as log contents',()=>{
  for(const text of ['C:\\logs\\device.log','"C:\\logs\\device.log"','\\\\server\\logs\\device.log','file:///Users/example/device.log','/Users/example/device.log','~/device.log','device.log','device.log\nsecond.log'])assert.equal(inspectPaste(event({getData:type=>type==='text/plain'?text:''})).message,FILE_PASTE_FALLBACK);
});
test('single threadtime and multiline text preserve original CRLF and spaces',()=>{
  for(const text of [log,'  '+log+'\r\nraw header\r\n']){const result=inspectPaste(event({getData:type=>type==='text/plain'?text:''}));assert.equal(result.kind,'text');assert.equal(result.text,text);assert.equal(result.size,new TextEncoder().encode(text).length);}
  assert.equal(inspectPaste(event({getData:()=> 'ordinary unrelated text'})).kind,'ignore');assert.equal(inspectPaste(event({getData:()=> ''})).kind,'ignore');
});
test('pasted text callback receives original bytes and size; URI-only and missing clipboard fall back',()=>{
  let received;const e=event({getData:type=>type==='text/plain'?log:''});handleLogPaste(e,{openFile:()=>assert.fail(),openText:(text,size)=>received={text,size},onError:()=>assert.fail()});assert.equal(received.text,log);assert.equal(e.prevented,true);
  assert.equal(inspectPaste(event({getData:type=>type==='text/uri-list'?'file:///tmp/synthetic.log':''})).kind,'error');assert.equal(inspectPaste({target:{},clipboardData:null}).kind,'ignore');
});
test('25MiB pasted text bound is checked by UTF-8 byte length',()=>{const text=log+'\n'+'가'.repeat(9*1024*1024);assert.equal(inspectPaste(event({getData:()=>text})).kind,'error');});
test('pasted text counts physical rows as the core splits them',()=>{
  const lines=text=>inspectPaste(event({getData:type=>type==='text/plain'?text:''})).lines;
  assert.equal(lines(log),1);assert.equal(lines(log+'\n'),1);assert.equal(lines(log+'\r\n'),1);
  assert.equal(lines('a\nb'),2);assert.equal(lines('a\r\nb\rc\n'),3);assert.equal(lines('a\n\nb\n\n'),4);
  let received;handleLogPaste(event({getData:()=> 'a\nb\n'}),{openFile:()=>assert.fail(),openText:(text,size,count)=>received={text,size,count},onError:()=>assert.fail()});
  assert.deepEqual(received,{text:'a\nb\n',size:4,count:2});
});
