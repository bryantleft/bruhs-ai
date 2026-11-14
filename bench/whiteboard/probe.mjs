// Pilot: identify completed-frame trace events before freezing retained runs.
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {pathToFileURL} from 'node:url';
import path from 'node:path';
const modulePath=process.env.BRUHS_PLAYWRIGHT_MODULE;
if(!modulePath)throw new Error('Set BRUHS_PLAYWRIGHT_MODULE to the installed Playwright module');
const {chromium}=await import(pathToFileURL(modulePath).href);
const [renderer='tldraw',countText='1000',directory='/tmp/bruhs-whiteboard-pilot',mode='normal']=process.argv.slice(2);
const count=Number(countText);
if(!['rust','tldraw'].includes(renderer)||![1000,10000].includes(count)||!['normal','uncapped'].includes(mode))throw new Error('Pass rust|tldraw 1000|10000 output-directory normal|uncapped');
await mkdir(directory,{recursive:true});
const scene=JSON.parse(await readFile(new URL(`scene-${count}.json`,import.meta.url),'utf8'));
const browser=await chromium.launch({executablePath:process.env.BRUHS_CHROME,headless:false,args:mode==='uncapped'?['--disable-frame-rate-limit']:[]});
try{
  const context=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:1});
  const page=await context.newPage();
  const errors=[],failedRequests=[];page.on('pageerror',error=>errors.push(error.message));
  page.on('requestfailed',request=>failedRequests.push({url:request.url(),failure:request.failure()}));
  page.on('console',message=>{if(message.type()==='error')errors.push(message.text());});
  await page.route('**/*',route=>{const url=new URL(route.request().url());return ['127.0.0.1','localhost'].includes(url.hostname)?route.continue():route.abort();});
  await page.goto(renderer==='rust'?'http://127.0.0.1:46167/whiteboard-benchmark':'http://127.0.0.1:46166/');
  await page.waitForFunction(()=>window.__boardBench?.ready());
  const initial=await page.evaluate(async scene=>{window.__neutralScene=scene;if(document.fonts.check('18px MonaspaceArgonVariable'))await document.fonts.load('18px MonaspaceArgonVariable');const result=await window.__boardBench.load(scene);await document.fonts.ready;await new Promise(requestAnimationFrame);await new Promise(requestAnimationFrame);return result;},scene);
  if(initial.shapes!==count||initial.bindings!==count/2)throw new Error(`Incomplete scene: ${JSON.stringify(initial)}`);
  await page.screenshot({path:path.join(directory,`${renderer}-${count}-${mode}-initial.png`)});
  const cdp=await context.newCDPSession(page),root=await browser.newBrowserCDPSession();
  const version=await root.send('Browser.getVersion');
  const system=await root.send('SystemInfo.getInfo');
  await cdp.send('Tracing.start',{categories:'benchmark,cc,viz,gpu,blink.user_timing,devtools.timeline,disabled-by-default-devtools.timeline.frame',transferMode:'ReturnAsStream'});
  const updates=await page.evaluate(async()=>{
    const api=window.__boardBench, rows=[];
    const frame=()=>new Promise(requestAnimationFrame);
    for(const phase of ['pan','single','group']){
      api.camera({x:0,y:0,zoom:1});
      if(phase!=='pan')api.begin(phase==='single'?['node-00000']:api.stats().geometry.map(shape=>shape.id).concat(Object.keys(window.__neutralScene.shapes).filter(id=>id.startsWith('arrow-'))));
      await frame();await frame();
      const began=performance.now();performance.mark(`board-${phase}-start`);
      let last=began,frames=0;
      while(performance.now()-began<2000){
        const timestamp=await frame(),t=Math.min(1,(timestamp-began)/2000);
        if(phase==='pan')api.camera({x:400*t,y:200*t,zoom:1});else api.movement({x:180*t,y:90*t});
        rows.push({phase,requested:timestamp,applied:performance.now(),interval:timestamp-last});last=timestamp;frames++;
      }
      performance.mark(`board-${phase}-end`);
      if(phase==='pan')api.camera({x:400,y:200,zoom:1});else{api.movement({x:180,y:90});api.end();}
      rows.push({phase,frames,final:api.stats()});
    }
    return rows;
  }).catch(error=>{throw new Error(`Pilot input failed: ${error.message}; page errors ${JSON.stringify(errors)}`);});
  const completion=new Promise(resolve=>cdp.once('Tracing.tracingComplete',resolve));
  await cdp.send('Tracing.end');
  const {stream}=await completion;
  if(!stream)throw new Error('Chromium did not return a trace stream');
  let trace='';
  while(true){const chunk=await cdp.send('IO.read',{handle:stream});trace+=chunk.data;if(chunk.eof)break;}
  await cdp.send('IO.close',{handle:stream});
  const events=JSON.parse(trace).traceEvents,names=new Map();
  for(const event of events)if(/frame|present|swap|pipeline/i.test(event.name))names.set(event.name,(names.get(event.name)||0)+1);
  await writeFile(path.join(directory,`${renderer}-${count}-${mode}.trace.json`),trace);
  await writeFile(path.join(directory,`${renderer}-${count}-${mode}.json`),JSON.stringify({initial,updates,version,system,errors,failedRequests,traceEvents:Object.fromEntries(names)},null,2));
  console.log(JSON.stringify({renderer,count,mode,errors,failedRequests,traceEvents:Object.fromEntries(names)}));
}finally{await browser.close();}
