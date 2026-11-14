// Production WASM with isolated storage and mocked database APIs. Rust tests
// separately cover real SQLite/Postgres, native HTTP calls, and agent grants.
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, readdir, stat } from 'node:fs/promises';
import { createServer } from 'node:http';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const dist=path.resolve(process.env.BRUHS_DATABASE_UI_DIST??path.join(root,'target/database-ui-dist'));
const output=path.join(root,'target/database-ui-qa');
await mkdir(output,{recursive:true});
assert((await stat(path.join(dist,'index.html'))).mtimeMs>=Math.max(...await Promise.all(['bruhs-web/src/components/database_panel.rs','bruhs-web/src/components/files_panel.rs','bruhs-models/src/database.rs'].map(async file=>(await stat(path.join(root,file))).mtimeMs))),'Refuse stale UI assets');
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
  const state={connections:[],calls:[],unexpected:[],errors:[],cancelledIds:new Set(),paged:false};
  const table={schema:'main',name:'customers',kind:'table'};
  const result=()=>({connection_id:state.connections[0].id,revision:state.connections[0].revision,columns:[{name:'id',data_type:'integer'},{name:'name',data_type:'text'},{name:'subscription',data_type:'text'}],rows:[['9223372036854775807','Ada',null],['2','<img src=x onerror="window.__dbXss=true">','pro']],truncated:false,elapsed_ms:3});
  await context.route('**/*',async route=>{
    const request=route.request(),url=new URL(request.url());
    if(!url.pathname.startsWith('/api/')) {if(url.origin===origin)return route.continue();state.unexpected.push(request.url());return route.abort();}
    const json=(value,status=200)=>route.fulfill({status,contentType:'application/json',headers:{'Access-Control-Allow-Origin':origin,'Access-Control-Allow-Headers':'*','Access-Control-Allow-Methods':'GET, POST, OPTIONS','Cache-Control':'no-store'},body:JSON.stringify(value)});
    if(request.method()==='OPTIONS')return json({});
    if(url.pathname==='/api/databases'&&request.method()==='GET')return json(state.connections);
    if(url.pathname==='/api/databases'&&request.method()==='POST') {
      const input=request.postDataJSON();state.calls.push({type:'save',input});
      if(input.target.type==='sqlite'&&input.target.path==='/missing')return json({error:'Choose an existing SQLite file.'},400);
      const previous=state.connections.find(connection=>connection.id===input.id);
      const connection={id:input.id,project_path:input.project_path,name:input.name,target:input.target,revision:(previous?.revision??0)+1,agent_access:previous?.agent_access??false,password_saved:Boolean(input.password)||previous?.password_saved||false,credential_version:randomUUID()};
      state.connections=state.connections.filter(value=>value.id!==connection.id);state.connections.push(connection);return json(connection);
    }
    if(url.pathname==='/api/databases/grant') {
      const input=request.postDataJSON();state.calls.push({type:'grant',input});const connection=state.connections.find(value=>value.id===input.id);assert.equal(connection.revision,input.expected_revision);connection.agent_access=input.enabled;connection.revision++;return json(connection);
    }
    if(/^\/api\/databases\/[^/]+\/tables$/.test(url.pathname))return json([table]);
    if(url.pathname==='/api/databases/describe')return json({...result(),columns:[{name:'name',data_type:'text'},{name:'type',data_type:'text'}],rows:[['id','INTEGER'],['name','TEXT'],['subscription','TEXT']]});
    if(url.pathname==='/api/databases/browse') {const input=request.postDataJSON();state.calls.push({type:'browse',input});let value=result();if(state.paged){const offset=input.browse.offset;value.rows=Array.from({length:2},(_,index)=>[String(offset+index+1),`Page row ${offset+index+1}`,null]);value.truncated=offset<4;value.next_offset=value.truncated?offset+2:null;}else if(input.browse.filter)value.rows=value.rows.filter(row=>row[1].includes(input.browse.filter));return json(value);}
    if(url.pathname==='/api/databases/query') {
      const input=request.postDataJSON();state.calls.push({type:'query',input});
      if(input.query.sql.startsWith('UPDATE'))return json({error:'Only a read-only statement is allowed.'},400);
      if(input.query.sql.includes('slow')) {while(!state.cancelledIds.has(input.query.request_id))await new Promise(resolve=>setTimeout(resolve,20));return json({error:'Query cancelled'},400);}
      return json(result());
    }
    if(url.pathname.startsWith('/api/databases/cancel/')) {state.calls.push({type:'cancel'});state.cancelledIds.add(url.pathname.split('/').at(-1));return route.fulfill({status:204,headers:{'Access-Control-Allow-Origin':origin},body:''});}
    if(url.pathname==='/api/agent/files/children')return json([]);
    if(url.pathname==='/api/agent/git-branches')return json({branches:[],current:null});
    if(url.pathname==='/api/agent/git-status')return json({branch:null,ahead:0,behind:0,dirty:[]});
    if(request.method()!=='GET') {state.unexpected.push(`${request.method()} ${url.pathname}`);return json({error:'Unexpected live action'},500);}
    if(url.pathname.endsWith('/stream'))return route.continue({url:`${origin}/__fixture-stream`});
    if(url.pathname==='/api/reactive')return json(empty());
    if(url.pathname==='/api/weekly-reports')return json({presets:[],reports:[],next_before:null});
    if(/\/(meetings|people|credentials|tasks|threads|skills|models|children)$/.test(url.pathname))return json([]);
    return json({});
  });
  const thread=randomUUID();
  await context.addInitScript(({thread,origin})=>{
    if(location.origin!==origin)return;
    const request=indexedDB.open('bruhs',1);
    request.onupgradeneeded=()=>request.result.createObjectStore('threads',{keyPath:'id'});
    request.onsuccess=()=>{const db=request.result;const tx=db.transaction('threads','readwrite');tx.objectStore('threads').put({id:thread,title:'Database fixture',project_name:'Fixture',project_path:'/fixture/project',updated_at:Date.now(),pinned:false,settled:false});tx.oncomplete=()=>db.close();tx.onabort=()=>db.close();};
    request.onerror=()=>{if(request.error?.name!=='VersionError')console.error('Synthetic thread seed failed',request.error?.name);};
  },{thread,origin});
  const page=await context.newPage();page.setDefaultTimeout(15000);page.on('pageerror',error=>{state.errors.push(error.message);console.error('Page error:',error.message);});
  await page.goto(`${origin}/#thread/${thread}`);
  await page.getByText('Database fixture',{exact:true}).first().waitFor();await dismissSidebar(page,cell.width);
  // The panel launcher and tabs use the production workspace; no component-only harness.
  await page.keyboard.press('Meta+Alt+b');
  if(!await page.getByRole('tab',{name:'Files',exact:true}).first().isVisible()) {await page.keyboard.press('Meta+d');}
  await page.getByRole('tab',{name:'Files',exact:true}).first().click();
  await page.getByRole('tab',{name:'Databases',exact:true}).click();
  const panel=page.locator('[data-database-panel]');await panel.getByRole('button',{name:'Connect',exact:true}).click();
  await panel.getByLabel('Connection name',{exact:true}).fill('Fixture SQLite');await panel.getByLabel('SQLite file on this machine',{exact:true}).fill('/missing');await panel.getByRole('button',{name:'Save connection',exact:true}).click();
  await panel.getByRole('alert').filter({hasText:'existing SQLite'}).waitFor();check(await panel.getByLabel('SQLite file on this machine').inputValue()==='/missing',`${cell.name}: a failed connection save retains input and visible error`);
  await panel.getByLabel('SQLite file on this machine').fill('/fixture/db.sqlite3');await panel.getByRole('button',{name:'Save connection',exact:true}).click();
  await panel.getByRole('switch',{name:'Allow agents to read this connection'}).waitFor();check(await panel.getByRole('switch').getAttribute('aria-checked')==='false',`${cell.name}: per-connection agent access starts off`);
  await panel.getByLabel('Schema / table',{exact:true}).selectOption('0');await panel.getByRole('button',{name:'Add selected to draft'}).waitFor();
  check(await panel.getByText('9223372036854775807',{exact:true}).count()===1,`${cell.name}: int64 display keeps text precision`);check(!await page.evaluate(()=>Boolean(window.__dbXss))&&await panel.locator('img').count()===0,`${cell.name}: database HTML stays inert`);
  if(cell.hasTouch){const hit=await panel.getByLabel('Select row 1',{exact:true}).evaluate(input=>({tag:input.parentElement.tagName,width:input.parentElement.getBoundingClientRect().width,height:input.parentElement.getBoundingClientRect().height}));check(hit.tag==='LABEL'&&hit.width>=44&&hit.height>=44,`${cell.name}: row selection has a 44px touch target`);}
  await panel.getByLabel('Select row 1',{exact:true}).check();await panel.getByRole('button',{name:'Add selected to draft'}).click();
  await panel.getByRole('status').filter({hasText:'current draft'}).waitFor();check(state.unexpected.length===0,`${cell.name}: attaching never sends a provider request (${JSON.stringify(state.unexpected)})`);
  const draft=await page.locator('#chat-input').inputValue();check(draft.includes('Ada')&&!draft.includes('<img'),`${cell.name}: only selected rows enter the unsent draft`);
  await panel.getByRole('button',{name:'Inspect row 1'}).click();check(await panel.getByLabel('Row detail').getByText('NULL',{exact:true}).count()===1,`${cell.name}: row detail preserves NULL`);await panel.getByRole('button',{name:'Close detail'}).click();
  await panel.getByLabel('Filter column',{exact:true}).fill('name');await panel.getByLabel('Contains text',{exact:true}).fill('Ada');await panel.getByLabel('Sort column',{exact:true}).fill('id');await panel.getByLabel('Descending',{exact:true}).check();await panel.getByRole('button',{name:'Browse rows',exact:true}).click();await panel.getByText('1 rows · 3 ms',{exact:true}).waitFor();
  const browse=state.calls.filter(call=>call.type==='browse').at(-1).input.browse;check(browse.filter_column==='name'&&browse.filter==='Ada'&&browse.sort_column==='id'&&browse.descending,`${cell.name}: sort and filter cross the typed API`);
  state.paged=true;
  await panel.getByLabel('Contains text',{exact:true}).fill('');await panel.getByLabel('Filter column',{exact:true}).fill('');await panel.getByLabel('Descending',{exact:true}).uncheck();
  await panel.getByRole('button',{name:'Browse rows',exact:true}).click();await panel.getByText('Page row 1',{exact:true}).waitFor();
  await panel.getByRole('button',{name:'Next rows',exact:true}).click();await panel.getByText('Page row 3',{exact:true}).waitFor();
  check(state.calls.filter(call=>call.type==='browse').at(-1).input.browse.offset===2,`${cell.name}: next page follows the returned byte-limited cursor`);
  await panel.getByRole('button',{name:'Previous rows',exact:true}).click();await panel.getByText('Page row 1',{exact:true}).waitFor();
  check(state.calls.filter(call=>call.type==='browse').at(-1).input.browse.offset===0,`${cell.name}: previous returns to the actual prior offset`);
  const readsBeforeDirty=state.calls.filter(call=>call.type==='browse').length;
  await panel.getByLabel('Descending',{exact:true}).check();check(await panel.getByRole('button',{name:'Next rows',exact:true}).isDisabled(),`${cell.name}: changed sort disables old-cursor paging`);
  check(state.calls.filter(call=>call.type==='browse').length===readsBeforeDirty,`${cell.name}: changing criteria alone does not issue a query`);
  await panel.getByRole('button',{name:'Browse rows',exact:true}).click();await panel.getByText('Page row 1',{exact:true}).waitFor();
  const restarted=state.calls.filter(call=>call.type==='browse').at(-1).input.browse;check(restarted.offset===0&&restarted.descending,`${cell.name}: new criteria restart at the first page`);
  await panel.locator('table').scrollIntoViewIfNeeded();
  await page.screenshot({path:path.join(output,`${cell.name}-results.png`)});
  state.paged=false;
  await panel.getByRole('button',{name:'Describe columns'}).click();await panel.getByText('INTEGER',{exact:true}).waitFor();
  await panel.getByLabel('Read-only SQL',{exact:true}).fill("UPDATE customers SET name='bad'");await panel.getByRole('button',{name:'Run query',exact:true}).click();await panel.getByRole('alert').filter({hasText:'read-only'}).waitFor();check((await panel.getByLabel('Read-only SQL').inputValue()).startsWith('UPDATE'),`${cell.name}: blocked SQL remains editable with an error`);
  await panel.getByLabel('Read-only SQL').fill('SELECT slow');await panel.getByRole('button',{name:'Run query',exact:true}).click();await panel.getByRole('button',{name:'Cancel query',exact:true}).click();await panel.getByRole('status').filter({hasText:'Query cancelled.'}).waitFor();check(state.calls.some(call=>call.type==='cancel'),`${cell.name}: cancelling calls the server and discards late results`);
  await panel.getByRole('switch').click();await panel.getByRole('switch').filter({hasText:'Agent access: on'}).waitFor();check(state.connections[0].agent_access,`${cell.name}: sharing requires an explicit owner action`);
  state.connections[0].revision+=1;state.connections[0].agent_access=false;
  await panel.getByRole('button',{name:'Refresh',exact:true}).click();await panel.getByRole('switch').filter({hasText:'Agent access: off'}).waitFor();
  await panel.getByLabel('Read-only SQL').fill('SELECT current revision');await panel.getByRole('button',{name:'Run query',exact:true}).click();await panel.getByRole('button',{name:'Add selected to draft'}).waitFor();
  check(state.calls.filter(call=>call.type==='query').at(-1).input.query.revision===state.connections[0].revision,`${cell.name}: refresh adopts the latest selected connection revision`);
  await panel.getByRole('button',{name:'Edit connection'}).click();await panel.getByLabel('Engine',{exact:true}).selectOption('postgres');await panel.getByLabel('Database',{exact:true}).fill('fixture');await panel.getByLabel('User',{exact:true}).fill('fixture_reader');await panel.getByLabel('Password (blank keeps saved value)',{exact:true}).fill('synthetic-private-value');await panel.getByRole('button',{name:'Save connection',exact:true}).click();await panel.getByRole('button',{name:'Edit connection'}).waitFor();await panel.getByRole('button',{name:'Edit connection'}).click();check(await panel.getByLabel('Password (blank keeps saved value)').inputValue()==='',`${cell.name}: saving clears the private field and edit never retrieves its value`);
  await panel.getByRole('button',{name:'Cancel',exact:true}).click();
  const geometry=await panel.evaluate(element=>({width:element.clientWidth,scroll:element.scrollWidth,viewport:innerWidth}));check(geometry.scroll<=geometry.width+1,`${cell.name}: result grid scrolls within the panel`);
  if(cell.hasTouch)check(await panel.getByLabel('Read-only SQL').evaluate(element=>parseFloat(getComputedStyle(element).fontSize))>=16,`${cell.name}: touch inputs avoid browser zoom`);
  await page.screenshot({path:path.join(output,`${cell.name}.png`)});
  check(state.errors.length===0,`${cell.name}: no browser runtime errors`);check(state.unexpected.length===0,`${cell.name}: only fixture APIs were used`);
  await context.close();
}
await import('node:fs/promises').then(fs=>fs.writeFile(path.join(output,'checks.json'),JSON.stringify({checks},null,2)));
console.log(JSON.stringify({passed:checks.length,checks},null,2));
} finally {await browser.close();await new Promise(resolve=>server.close(resolve));}

