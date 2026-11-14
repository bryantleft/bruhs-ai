// Production WASM, isolated browser storage and mocked API responses. The live
// provider/SQLite delivery has a separate opt-in Rust integration test.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, readdir, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const dist=path.resolve(process.env.BRUHS_WEEKLY_UI_DIST??path.join(root,'target/weekly-reports-ui-dist'));
const output=path.join(root,'target/weekly-reports-ui-qa');
await mkdir(output,{recursive:true});
assert((await stat(path.join(dist,'index.html'))).mtimeMs>=Math.max(...await Promise.all(['bruhs-web/src/components/reactive_automations.rs','bruhs-web/src/components/reactive_automations/weekly_reports.rs'].map(async file=>(await stat(path.join(root,file))).mtimeMs))),'Refuse stale UI assets');
let playwright;
try {playwright=await import('playwright');}catch {
  for(const entry of await readdir(path.join(homedir(),'.npm/_npx'))) {try {playwright=await import(pathToFileURL(path.join(homedir(),'.npm/_npx',entry,'node_modules/playwright/index.mjs')).href);break;}catch {}}
}
assert(playwright,'An existing Playwright installation is required');
const server=createServer(async(req,res)=>{
  const url=new URL(req.url,'http://127.0.0.1');
  if(url.pathname==='/__fixture-stream') {res.writeHead(200,{'Content-Type':'text/event-stream','Access-Control-Allow-Origin':'*'});res.write(': fixture\n\n');const timer=setInterval(()=>res.write(': connected\n\n'),1000);req.on('close',()=>clearInterval(timer));return;}
  const file=path.resolve(dist,`.${url.pathname==='/'?'/index.html':decodeURIComponent(url.pathname)}`);
  if(!file.startsWith(dist+path.sep)){res.writeHead(403);res.end();return;}
  try {const bytes=await readFile(file);res.writeHead(200,{'Content-Type':{'.html':'text/html','.js':'text/javascript','.wasm':'application/wasm','.css':'text/css','.svg':'image/svg+xml','.woff2':'font/woff2'}[path.extname(file)]??'application/octet-stream'});res.end(bytes);}catch {res.writeHead(404);res.end();}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const origin=`http://127.0.0.1:${server.address().port}`;
const browser=await playwright.chromium.launch({headless:true,executablePath:process.env.BRUHS_CHROME??(process.platform==='darwin'?'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome':undefined)});
const checks=[];
const check=(ok,label)=>{assert(ok,label);checks.push(label);};
const empty=()=>({agents:[],sources:[],subscriptions:[],events:[],runs:[],proposals:[],source_statuses:[]});
async function dismissSidebar(page,width) {
  const scrim=page.locator('div[aria-hidden="true"].fixed.inset-0');
  if(await scrim.isVisible()) {await scrim.click({position:{x:width-12,y:60}});await page.getByRole('complementary',{name:'Sidebar navigation',exact:true}).waitFor({state:'hidden'});}
}

try {
for(const cell of [{name:'compact',width:640,height:480,hasTouch:false},{name:'touch',width:390,height:844,hasTouch:true}]) {
  const context=await browser.newContext({viewport:{width:cell.width,height:cell.height},hasTouch:cell.hasTouch,serviceWorkers:'block'});
  const state={presets:[],reports:[],commands:[],reads:0,unexpected:[],errors:[],runtime:empty()};
  const snapshot=()=>({presets:state.presets,reports:state.reports.map(({id,preset_id,title,project_path,window_end,revision})=>({id,preset_id,title,project_path,window_end,revision})),next_before:null});
  await context.route('**/*',async route=>{
    const request=route.request(),url=new URL(request.url());
    if(!url.pathname.startsWith('/api/')) {if(url.origin===origin)return route.continue();state.unexpected.push(request.url());return route.abort();}
    const json=(value,status=200)=>route.fulfill({status,contentType:'application/json',headers:{'Access-Control-Allow-Origin':origin,'Access-Control-Allow-Headers':'*','Access-Control-Allow-Methods':'GET, POST, OPTIONS','Cache-Control':'no-store'},body:JSON.stringify(value)});
    if(request.method()==='OPTIONS')return json({});
    if(url.pathname==='/api/weekly-reports'&&request.method()==='GET'){state.reads++;return json(snapshot());}
    if(url.pathname==='/api/weekly-reports/commands') {
      const command=request.postDataJSON();state.commands.push(command);
      await new Promise(resolve=>setTimeout(resolve,150));
      if(command.action==='save_preset') {
        if(command.preset.project_path==='/missing')return json({error:'Choose an existing absolute folder path.'},400);
        const index=state.presets.findIndex(preset=>preset.id===command.preset.id);if(index<0)state.presets.push(command.preset);else state.presets[index]=command.preset;
      }else if(command.action==='run_now') {
        assert.match(command.request_id,/^[0-9a-f-]{36}$/);
      }else if(command.action==='edit_report') {
        const report=state.reports.find(report=>report.id===command.id);assert(report);
        if(command.expected_revision!==report.revision)return json({error:'This report changed in another window. Reload it before saving your edits.'},409);
        report.body=command.body;report.title=command.title;report.revision++;
      }else assert.fail(`Unexpected report action ${command.action}`);
      return json(snapshot());
    }
    if(url.pathname.startsWith('/api/weekly-reports/')&&request.method()==='GET') {const report=state.reports.find(report=>report.id===url.pathname.split('/').at(-1));return report?json(report):json({error:'Missing report'},404);}
    if(url.pathname==='/api/reactive'&&request.method()==='GET')return json(state.runtime);
    if(request.method()!=='GET') {state.unexpected.push(`${request.method()} ${url.pathname}`);return json({error:'Unexpected live action'},500);}
    if(url.pathname.endsWith('/stream'))return route.continue({url:`${origin}/__fixture-stream`});
    if(/\/(meetings|people|credentials|tasks|threads|skills|models)$/.test(url.pathname))return json([]);
    return json({});
  });
  const page=await context.newPage();page.on('pageerror',error=>state.errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.goto(`${origin}/#/automations`);
  const panel=page.locator('section[aria-labelledby="weekly-reports-title"]');
  await panel.getByRole('button',{name:'New report preset',exact:true}).waitFor();
  await page.waitForFunction(()=>!document.querySelector('section[aria-labelledby="reactive-title"] > fieldset')?.disabled);
  await dismissSidebar(page,cell.width);
  await panel.getByRole('button',{name:'New report preset',exact:true}).click();
  check(await panel.getByLabel('Local Git commits',{exact:true}).isChecked(),`${cell.name}: Git starts selected`);
  check(!await panel.getByLabel('Repeat every seven days',{exact:true}).isChecked(),`${cell.name}: unattended scheduling starts off`);
  await panel.getByLabel('Report project folder',{exact:true}).fill('/missing');
  await panel.getByLabel('Report model',{exact:true}).fill('claude-haiku-4-5');
  await panel.getByRole('button',{name:'Save report preset',exact:true}).click();
  await panel.getByRole('alert').filter({hasText:'Choose an existing'}).waitFor();
  await page.waitForResponse(response=>new URL(response.url()).pathname==='/api/weekly-reports'&&response.request().method()==='GET'&&response.status()===200);
  check(await panel.getByRole('alert').filter({hasText:'Choose an existing'}).isVisible(),`${cell.name}: failed save stays visible through polling`);
  check(await panel.getByLabel('Report project folder',{exact:true}).inputValue()==='/missing',`${cell.name}: failed save retains the draft`);
  await panel.getByLabel('Report project folder',{exact:true}).fill('/fixture/report project');
  await panel.getByLabel('GitHub pull requests',{exact:true}).check();await panel.getByLabel('GitHub deployments',{exact:true}).check();
  await panel.getByLabel('Repeat every seven days',{exact:true}).check();
  const beforeSave=state.commands.length;
  await panel.getByRole('button',{name:'Save report preset',exact:true}).evaluate(button=>{button.click();button.click();});
  await panel.getByText('Report preset saved.',{exact:true}).waitFor();
  check(state.commands.length===beforeSave+1,`${cell.name}: duplicate submit produces one preset command`);
  const preset=state.presets[0];assert(preset.local_git&&preset.github_pull_requests&&preset.github_deployments&&preset.scheduled);assert.equal(preset.model,'claude-haiku-4-5');
  await panel.getByLabel('Report window end in UTC (optional)',{exact:true}).fill('2026-11-06T15:00:00Z');
  await panel.getByRole('button',{name:'Generate report now',exact:true}).click();
  await panel.getByText('Report queued. Follow its run in activity below.',{exact:true}).waitFor();
  check(state.commands.at(-1).window_end_utc==='2026-11-06T15:00:00Z',`${cell.name}: selected reporting window reaches the host`);
  const report={id:randomUUID(),preset_id:preset.id,run_id:randomUUID(),thread_id:randomUUID(),project_path:preset.project_path,model:preset.model,window_start:Date.parse(state.commands.at(-1).window_end_utc)-7*24*60*60*1000,window_end:Date.parse(state.commands.at(-1).window_end_utc),title:'Engineering report fixture',body:'# Repository changes\nOne synthetic commit.\n\n<img src="https://unrelated.invalid/tracker">',coverage:[{status:'available',source:'Local Git commits',facts:[{title:'Synthetic commit',reference:'commit abc',url:'https://github.com/example/project/commit/abc',occurred_at:1794580000000}],truncated:false},{status:'unavailable',source:'Incidents',reason:'No feed is connected. No zero count can be inferred.'}],created_at:1794585600000,updated_at:1794585600000,revision:1};
  state.reports.push(report);
  await panel.getByRole('button',{name:'Open report',exact:true}).waitFor();
  await panel.getByRole('button',{name:'Open report',exact:true}).click();
  await panel.getByLabel('Report Markdown',{exact:true}).waitFor();
  check(await panel.getByLabel('Report Markdown',{exact:true}).inputValue()===report.body,`${cell.name}: report reopens with its saved body`);
  await panel.getByText('Sources and missing feeds',{exact:true}).click();
  check(await panel.getByText('Incidents: No feed is connected. No zero count can be inferred.',{exact:true}).isVisible(),`${cell.name}: missing feed is explicit`);
  check(await panel.getByRole('link',{name:'Synthetic commit (commit abc)',exact:true}).getAttribute('rel')==='noopener noreferrer',`${cell.name}: source links keep the external-link boundary`);
  await panel.getByLabel('Report title',{exact:true}).fill('Owner reviewed report');
  await panel.getByLabel('Report Markdown',{exact:true}).fill('Owner edits retained through polling.');
  await page.waitForTimeout(2200);
  check(await panel.getByLabel('Report Markdown',{exact:true}).inputValue()==='Owner edits retained through polling.',`${cell.name}: refresh preserves unsaved text`);
  await panel.getByRole('button',{name:'Save report edits',exact:true}).click();
  await panel.getByText('Report saved.',{exact:true}).waitFor();
  assert.equal(report.revision,2);assert.equal(report.body,'Owner edits retained through polling.');
  await page.reload();await panel.getByRole('button',{name:'Open report',exact:true}).waitFor();await dismissSidebar(page,cell.width);await panel.getByRole('button',{name:'Open report',exact:true}).click();
  await panel.getByLabel('Report Markdown',{exact:true}).waitFor();
  check(await panel.getByLabel('Report Markdown',{exact:true}).inputValue()===report.body,`${cell.name}: saved edits survive reload`);
  report.revision++;
  await panel.getByLabel('Report Markdown',{exact:true}).fill('Local conflicting edit');
  await panel.getByRole('button',{name:'Save report edits',exact:true}).click();
  await panel.getByRole('alert').filter({hasText:'changed in another window'}).waitFor();await page.waitForTimeout(2200);
  check(await panel.getByLabel('Report Markdown',{exact:true}).inputValue()==='Local conflicting edit',`${cell.name}: conflict preserves the local draft`);
  check(await panel.getByRole('alert').filter({hasText:'changed in another window'}).isVisible(),`${cell.name}: conflict survives refresh`);
  await panel.getByRole('button',{name:'Open project and agent thread',exact:true}).click();
  await page.getByLabel('Message to agent',{exact:true}).waitFor();
  check(await page.getByLabel('Message to agent',{exact:true}).inputValue()===report.body,`${cell.name}: thread opens with the selected saved report as an unsent draft`);
  check(await page.evaluate(()=>JSON.parse(localStorage.getItem('bruhs.activeProject')).path)===preset.project_path,`${cell.name}: thread action restores the report project`);
  check(await panel.evaluate(element=>element.scrollWidth<=element.clientWidth+1),`${cell.name}: report controls do not overflow horizontally`);
  if(cell.hasTouch)check(Number.parseFloat(await panel.getByLabel('Report Markdown',{exact:true}).evaluate(input=>getComputedStyle(input).fontSize))>=16,`${cell.name}: touch editor uses readable text size`);
  await panel.getByRole('heading',{name:'Edit saved report',exact:true}).scrollIntoViewIfNeeded();
  const bounds=await panel.boundingBox();
  check(bounds.x>=0&&bounds.x+bounds.width<=cell.width+1,`${cell.name}: report stays inside the viewport`);
  await page.screenshot({path:path.join(output,`${cell.name}-report-editor.png`)});
  check(state.errors.length===0,`${cell.name}: no browser runtime errors`);
  check(state.unexpected.length===0,`${cell.name}: no remote content fetch or live mutation`);
  await context.close();
}
await (await import('node:fs/promises')).writeFile(path.join(output,'results.json'),JSON.stringify({checks},null,2)+'\n');
console.log(JSON.stringify({passed:checks.length,artifacts:output}));
} finally {await browser.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
