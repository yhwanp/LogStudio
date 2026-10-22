// Sends one engine request over Tauri IPC. Engine error text becomes
// userMessage; an aborted request rejects with AbortError even if Rust replies.
export function desktopTransport(tauri){
 const invoke=tauri.core.invoke,aborted=()=>new DOMException('Aborted','AbortError');
 return async (request,signal)=>{
  if(signal?.aborted)throw aborted();
  let abort;
  try{
   const pending=invoke('log_request',{request});
   const data=await (signal?Promise.race([pending,new Promise((_,reject)=>{abort=()=>reject(aborted());signal.addEventListener('abort',abort,{once:true});})]):pending);
   if(signal?.aborted)throw aborted();return data;
  }catch(error){
   if(signal?.aborted)throw aborted();
   throw Object.assign(new Error('Local request failed'),{userMessage:String(error).slice(0,500)});
  }finally{if(abort)signal.removeEventListener('abort',abort);}
 };
}
