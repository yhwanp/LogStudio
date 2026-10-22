import {MAX_BYTES} from './filter-model.js';
export const FILE_PASTE_FALLBACK='파일은 붙여넣기로 열 수 없습니다. 로그 파일 열기나 끌어다 놓기로 열어 주세요.';
export function isEditableTarget(target) {
  return !!(target?.isContentEditable || target?.closest?.('input,textarea,[contenteditable]:not([contenteditable="false"]),[role="textbox"]'));
}
function isFileReference(text) {
  const lines=text.trim().split(/\r\n|\r|\n/).filter(line=>line.trim()&&!line.trim().startsWith('#'));
  return lines.length>0&&lines.every(line=>{
    const value=line.trim().replace(/^(["'])(.*)\1$/,'$2');
    return /^(?:file:\/\/|[A-Za-z]:[\\/]|\\\\|\/|~\/)/i.test(value)||/^[^\r\n\/\\]+\.(?:log|txt|dlt)$/i.test(value);
  });
}
// Called only from a user-initiated paste event. Never poll navigator.clipboard.
export function inspectPaste(event) {
  if(isEditableTarget(event.target))return{kind:'ignore'};
  const data=event.clipboardData;
  if(!data)return{kind:'ignore'};
  let files=Array.from(data.files??[]);
  const fileItems=Array.from(data.items??[]).filter(item=>item.kind==='file');
  if(!files.length)files=fileItems.map(item=>item.getAsFile?.()).filter(Boolean);
  if(files.length>1)return{kind:'error',message:'한 번에 로그 파일 하나만 붙여넣어 주세요.'};
  if(files.length===1)return{kind:'file',file:files[0]};
  if(fileItems.length)return{kind:'error',message:FILE_PASTE_FALLBACK};
  const text=data.getData?.('text/plain')||data.getData?.('text/uri-list')||'';
  if(!text.trim())return{kind:'ignore'};
  if(isFileReference(text))return{kind:'error',message:FILE_PASTE_FALLBACK};
  const lines=text.split(/\r\n|\r|\n/);
  if(lines.length<2&&!/^\s*(?:\d+\s+)?(?:\d{2}-\d{2}|\d{4}-\d{2}-\d{2})[\t ]+\d{2}:\d{2}:\d{2}\./.test(text))return{kind:'ignore'};
  const size=new TextEncoder().encode(text).length;
  if(size>MAX_BYTES)return{kind:'error',message:'최대 25 MiB까지 붙여넣을 수 있습니다. 로그 내용을 나누어 다시 붙여넣어 주세요.'};
  // Physical rows, as the core splits them: a final line break ends the last row.
  return{kind:'text',text,size,lines:lines.length-(lines.length>1&&lines.at(-1)===''?1:0)};
}
export function handleLogPaste(event,{openFile,openText,onError}) {
  const result=inspectPaste(event);
  if(result.kind==='ignore')return false;
  event.preventDefault();
  if(result.kind==='file')openFile(result.file);
  else if(result.kind==='text')openText(result.text,result.size,result.lines);
  else onError(result.message);
  return true;
}
