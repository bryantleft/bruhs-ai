// Metadata-only browser fixture. It never reads CLI logs, credentials or model APIs.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
let sequence=0,preview=null,lastCommitted=null,records=0,inventory='inventory-a',delayMs=0,commitError=false;
const calls=[];let config={acp:[],prices:{}};
const imported=()=>({records,by_model:records?{'fixture-model':{turns:records,input:100,output:20,cache_read:30,cache_write:0,estimated_usd:null}}:{},by_project:{},by_day:{},imported_at:records?1:null});
const server=createServer(async(req,res)=>{
 try{
  const chunks=[];for await(const chunk of req)chunks.push(chunk);
  const body=chunks.length?JSON.parse(Buffer.concat(chunks).toString()):undefined;
  if(req.url.startsWith('/ui')){res.setHeader('content-type','text/html');res.end(await readFile(new URL('../../bruhs-server/src/routes/agent_extensions.html',import.meta.url)));return;}
  if(req.url==='/__control'){if(body.inventory!==undefined)inventory=body.inventory;if(body.delayMs!==undefined)delayMs=body.delayMs;if(body.commitError!==undefined)commitError=body.commitError;res.end('{}');return;}
  if(req.url!=='/__fixture')calls.push({url:req.url,body});
  let response;
  if(req.url==='/__fixture')response={calls,preview,lastCommitted,records,inventory};
  else if(req.url==='/api/agent-extensions/config'){if(body)config=body;response=config;}
  else if(req.url==='/api/agent-extensions/usage')response={by_model:{},by_project:{},by_day:{},limits:[],errors:[],imported:imported()};
  else if(req.url==='/api/agent-extensions/history/preview'){
   if(body.offset>0&&body.inventory!==inventory){res.statusCode=400;response={error:'The native log file inventory changed. Restart from the first page.'};}
   else{const capturedInventory=inventory,delay=delayMs;delayMs=0;await new Promise(resolve=>setTimeout(resolve,delay));preview={id:'preview-'+(++sequence),source:body.source,root:'/fixture/'+('a'.repeat(160)),inventory:capturedInventory,files_read:1,bytes_read:400,next_offset:body.offset===0?1:null,minimum_next_bytes:null,limited:false,warnings:[],usage:{...imported(),records:1,by_model:{[body.source+'-fixture']:{turns:1,input:100,output:20,cache_read:30,cache_write:0,estimated_usd:null}}}};response=preview;}
  }else if(/^\/api\/agent-extensions\/history\/[^/]+\/commit$/.test(req.url)){
   const id=decodeURIComponent(req.url.split('/').at(-2));
   if(commitError){res.statusCode=409;response={error:'Fixture commit rejected; no metadata was imported.'};}
   else if(id===lastCommitted)response=imported();
   else if(!preview||preview.id!==id){res.statusCode=409;response={error:'This usage preview was replaced. Review it again.'};}
   else{lastCommitted=id;preview=null;records++;response=imported();}
  }else{res.statusCode=404;response={error:'Unknown fixture route'};}
  res.setHeader('content-type','application/json');res.end(JSON.stringify(response));
 }catch(error){res.statusCode=500;res.end(JSON.stringify({error:error.message}));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
console.log(JSON.stringify({origin:'http://127.0.0.1:'+server.address().port,ui:'http://127.0.0.1:'+server.address().port+'/ui'}));
await new Promise(resolve=>process.on('SIGTERM',resolve));server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
