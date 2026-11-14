// Frozen paired rendering experiment. Input callbacks are diagnostics, not frames.
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {pathToFileURL,fileURLToPath} from 'node:url';
import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {gzip} from 'node:zlib';
import {createHash} from 'node:crypto';
import path from 'node:path';
import os from 'node:os';
const execute=promisify(execFile),compress=promisify(gzip);
const root=path.dirname(fileURLToPath(import.meta.url));
const modulePath=process.env.BRUHS_PLAYWRIGHT_MODULE;
if(!modulePath||!process.env.BRUHS_CHROME)throw new Error('Set BRUHS_PLAYWRIGHT_MODULE and BRUHS_CHROME to the installed executable/module paths.');
const {chromium}=await import(pathToFileURL(modulePath).href);
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const required=(condition,message)=>{if(!condition)throw new Error(message);};
async function thermal(){try{return(await execute('/usr/bin/pmset',['-g','therm'])).stdout.trim();}catch(error){return `unavailable: ${error.message}`;}}
function validate(scene,stats,phase){
  const count=Object.keys(scene.shapes).length;
  required(stats.shapes===count&&stats.bindings===count/2,'Shape or binding count changed');
  const expectedCamera=phase==='pan'?{x:400,y:200,zoom:1}:{x:0,y:0,zoom:1};
  for(const key of ['x','y','zoom'])required(Math.abs(stats.camera[key]-expectedCamera[key])<0.0001,`Incorrect ${phase} camera ${key}`);
  const geometry=new Map(stats.geometry.map(shape=>[shape.id,shape]));
  const position=id=>{const shape=scene.shapes[id];let x=shape.rect.x,y=shape.rect.y;if(phase!=='pan'&&id==='node-00000'){x+=180;y+=90;}if(phase==='group'){x+=180;y+=90;}return{x,y};};
  for(const shape of Object.values(scene.shapes)){
    if(shape.kind.type==='arrow')continue;
    const actual=geometry.get(shape.id),expected=position(shape.id);
    required(actual&&Math.abs(actual.x-expected.x)<0.0001&&Math.abs(actual.y-expected.y)<0.0001,`${phase} omitted or moved node ${shape.id} incorrectly`);
  }
  required(Array.isArray(stats.arrows)&&stats.arrows.length===count/4,'Missing actual arrow terminal geometry');
  const arrows=new Map(stats.arrows.map(arrow=>[arrow.id,arrow]));
  for(const shape of Object.values(scene.shapes).filter(s=>s.kind.type==='arrow')){
    const actual=arrows.get(shape.id);required(actual,`Missing arrow ${shape.id}`);
    for(const terminal of ['start','end']){
      const binding=shape.kind[terminal].binding;required(binding,'Neutral arrow is unbound');
      const target=scene.shapes[binding.shape_id],origin=position(target.id);
      const expected={x:origin.x+target.rect.w*binding.anchor.x,y:origin.y+target.rect.h*binding.anchor.y};
      // The SDK clamps exact boundary anchors inward by 0.001 of node size.
      required(Math.hypot(actual[terminal].x-expected.x,actual[terminal].y-expected.y)<=0.25,`${phase} arrow ${shape.id} ${terminal} did not follow its bound node`);
    }
  }
  return{phase,shapes:count,bindings:stats.bindings,validatedNodes:geometry.size,validatedArrows:arrows.size};
}
async function member({renderer,count,mode,directory,replay=false}){
  const prefix=path.join(directory,`member-${renderer}`);
  const scene=JSON.parse(await readFile(path.join(root,`scene-${count}.json`),'utf8'));
  const browser=await chromium.launch({executablePath:process.env.BRUHS_CHROME,headless:false,args:mode==='uncapped'?['--disable-frame-rate-limit']:[]});
  let cdp,traceStarted=false,trace,rows,initial,version,gpu,fonts,validation=[],failure;
  const errors=[],requests=[];const thermalBefore=await thermal();
  try{
    const context=await browser.newContext({viewport:{width:1280,height:800},deviceScaleFactor:1});
    const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));page.on('console',e=>{if(e.type()==='error')errors.push(e.text());});
    page.on('requestfailed',r=>errors.push(`${r.url()}: ${r.failure()?.errorText}`));
    page.on('response',r=>requests.push({url:r.url(),status:r.status()}));
    const base=renderer==='rust'?'http://127.0.0.1:46167':'http://127.0.0.1:46166';
    await page.route('**/*',route=>new URL(route.request().url()).origin===base?route.continue():route.abort());
    await page.goto(base+(renderer==='rust'?'/whiteboard-benchmark':'/'));
    await page.waitForFunction(()=>window.__boardBench?.ready());
    initial=await page.evaluate(async scene=>{
      window.__neutralScene=scene;
      const api=window.__boardBench;
      if(document.querySelector('#whiteboard-canvas')){const faces=await document.fonts.load('18px MonaspaceArgonVariable');if(!faces.length)throw new Error('Rust canvas font face is not registered');}
      await api.load(scene);await document.fonts.ready;
      await new Promise(requestAnimationFrame);await new Promise(requestAnimationFrame);
      const began=performance.now();while(performance.now()-began<500){const stamp=await new Promise(requestAnimationFrame),t=(stamp-began)/500;api.camera({x:100*t,y:50*t,zoom:1});}
      api.camera({x:0,y:0,zoom:1});await new Promise(requestAnimationFrame);await new Promise(requestAnimationFrame);
      return api.stats();
    },scene);
    required(initial.shapes===count&&initial.bindings===count/2,'The renderer omitted fixture records');
    fonts=await page.evaluate(()=>[...document.fonts].map(f=>({family:f.family,status:f.status,weight:f.weight,style:f.style})));
    required(requests.some(r=>r.url.endsWith('/fonts/MonaspaceArgonVariable.woff2')&&r.status===200),'The common local font was not fetched successfully');
    required(errors.length===0,`Renderer initialization errors: ${errors.join('; ')}`);
    if(replay)await page.screenshot({path:`${prefix}-initial.png`});
    cdp=await context.newCDPSession(page);const rootSession=await browser.newBrowserCDPSession();
    version=await rootSession.send('Browser.getVersion');const system=await rootSession.send('SystemInfo.getInfo');gpu={devices:system.gpu.devices,featureStatus:system.gpu.featureStatus};
    if(!replay){await cdp.send('Tracing.start',{categories:'benchmark,cc,viz,gpu,blink.user_timing,devtools.timeline,disabled-by-default-devtools.timeline.frame',transferMode:'ReturnAsStream'});traceStarted=true;}
    rows=await page.evaluate(async()=>{
      const api=window.__boardBench,scene=window.__neutralScene,rows=[];
      const frame=()=>new Promise(requestAnimationFrame);
      for(const phase of ['pan','single','group']){
        api.camera({x:0,y:0,zoom:1});if(phase!=='pan')api.begin(phase==='single'?['node-00000']:Object.keys(scene.shapes));
        await frame();await frame();const before=api.stats().updates;
        const began=performance.now();performance.mark(`board-${phase}-start`);let frames=0,last=began;const input=[];
        while(performance.now()-began<2000){const timestamp=await frame(),t=Math.max(0,Math.min(1,(timestamp-began)/2000));const entered=performance.now();if(phase==='pan')api.camera({x:400*t,y:200*t,zoom:1});else api.movement({x:180*t,y:90*t});input.push({requested:timestamp,handlerStart:entered,handlerEnd:performance.now(),rafInterval:timestamp-last});last=timestamp;frames++;}
        performance.mark(`board-${phase}-end`);
        if(phase==='pan')api.camera({x:400,y:200,zoom:1});else{api.movement({x:180,y:90});api.end();}
        await frame();await frame();const final=api.stats();
        if(final.updates-before!==frames+1)throw new Error('The application did not acknowledge every dispatched update');
        rows.push({phase,frames,updates:final.updates-before,input,final});
      }return rows;
    });
    validation=rows.map(row=>validate(scene,row.final,row.phase));
    required(errors.length===0,`Measured renderer errors: ${errors.join('; ')}`);
    if(replay)await page.screenshot({path:`${prefix}-group-final.png`});
  }catch(error){failure=error.stack||String(error);}
  finally{
    if(traceStarted&&cdp){try{const completion=new Promise(resolve=>cdp.once('Tracing.tracingComplete',resolve));await cdp.send('Tracing.end');const completed=await completion;required(!completed.dataLossOccurred,'Chromium dropped trace data');required(completed.stream,'Chromium returned no trace stream');trace='';while(true){const chunk=await cdp.send('IO.read',{handle:completed.stream});trace+=chunk.data;if(chunk.eof)break;}await cdp.send('IO.close',{handle:completed.stream});}catch(error){failure=[failure,error.stack].filter(Boolean).join('\n');}}
    await browser.close();
  }
  const metadata={schema:1,renderer,count,mode,replay,initial,rows,validation,errors,requests,fonts,version,gpu,thermalBefore,thermalAfter:await thermal(),failure};
  await writeFile(`${prefix}.json`,JSON.stringify(metadata,null,2),{flag:'wx'});
  if(trace){await writeFile(`${prefix}.trace.json.gz`,await compress(trace,{level:3}),{flag:'wx'});}
  required(!failure,failure||'The benchmark failed');
  if(!replay){const {stdout}=await execute('python3',[path.join(root,'analyze.py'),`${prefix}.trace.json.gz`]);const summary={renderer,count,mode,...JSON.parse(stdout)};await writeFile(`${prefix}.summary.json`,JSON.stringify(summary,null,2),{flag:'wx'});return summary;}
  return{renderer,count,mode,replay,validation};
}
const [cohort,directory,only]=process.argv.slice(2);
required(['primary','normal','scale','replay','pilot'].includes(cohort)&&directory,'Pass primary|normal|scale|replay|pilot and a NEW output directory');
required(!only||cohort==='pilot'&&['rust','tldraw'].includes(only),'A single renderer is allowed only in excluded pilot runs');
await mkdir(directory,{recursive:false});
const count=cohort==='scale'?10000:1000,mode=cohort==='normal'?'normal':'uncapped';
const pairs=cohort==='primary'?12:cohort==='normal'?3:cohort==='scale'?2:1;
const buildFiles=[path.join(root,'protocol.md'),path.join(root,`scene-${count}.json`),path.join(root,'run.mjs'),path.join(root,'analyze.py'),path.join(root,'comparator/package-lock.json'),path.join(root,'comparator/dist/src.js'),path.join(root,'comparator/dist/fonts/MonaspaceArgonVariable.woff2')];
const rustDist=path.resolve(process.env.BRUHS_WHITEBOARD_DIST??path.join(root,'../../target/whiteboard-ui-dist'));
const {readdir}=await import('node:fs/promises');
for(const file of await readdir(rustDist))if(/\.(wasm|js|css|html)$/.test(file))buildFiles.push(path.join(rustDist,file));
for(const file of await readdir(path.join(root,'comparator/dist')))if(/\.(js|css|html)$/.test(file))buildFiles.push(path.join(root,'comparator/dist',file));
buildFiles.push(path.join(root,'comparator/src.tsx'),path.join(root,'comparator/build.mjs'));
const hashes={};for(const file of buildFiles)hashes[path.relative(root,file)]=sha(await readFile(file));
let hardware;try{hardware=(await execute('/usr/sbin/sysctl',['hw.model','hw.physicalcpu','hw.logicalcpu','hw.memsize','machdep.cpu.brand_string'])).stdout.trim();}catch(error){hardware=error.message;}
await writeFile(path.join(directory,'manifest.json'),JSON.stringify({schema:1,cohort,count,mode,pairs,started:new Date().toISOString(),os:{platform:os.platform(),release:os.release(),arch:os.arch()},hardware,hashes},null,2),{flag:'wx'});
for(let pair=0;pair<pairs;pair++){
  const target=path.join(directory,`pair-${String(pair).padStart(2,'0')}`);await mkdir(target);
  const order=pair%2===0?['rust','tldraw']:['tldraw','rust'];
  for(const renderer of order.filter(renderer=>!only||renderer===only)){const result=await member({renderer,count,mode,directory:target,replay:cohort==='replay'});console.log(JSON.stringify({pair,renderer,score_ms:result.score_ms,workloads:result.workloads,validation:result.validation}));}
}
if(cohort==='primary'){const {stdout}=await execute('python3',[path.join(root,'analyze.py'),directory,'--pairs']);await writeFile(path.join(directory,'decision.json'),stdout,{flag:'wx'});console.log(stdout);}
for(const [file,expected] of Object.entries(hashes))required(sha(await readFile(path.resolve(root,file)))===expected,`Measured artifact changed during the cohort: ${file}`);
await writeFile(path.join(directory,'completion.json'),JSON.stringify({completed_at:new Date().toISOString(),artifacts_unchanged:true},null,2),{flag:'wx'});
