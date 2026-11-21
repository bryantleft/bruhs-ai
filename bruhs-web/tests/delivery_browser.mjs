// Offline UI acceptance: no GitHub credentials, model calls, or repository writes.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
const html = await readFile(new URL('../../bruhs-server/src/routes/delivery_ui.html', import.meta.url));
const workbenchHtml = await readFile(new URL('../../bruhs-server/src/routes/workbench.html', import.meta.url));
const coordinationHtml = await readFile(new URL('../../bruhs-server/src/routes/coordination/remote.html', import.meta.url));
const calls = [];
let captureWriterRequest, captureGuideRequest, guidesEnabled, cachedGuide = null;
let proposals = [], receipts = [], watches = [], trackers = [];
const pull = {number:42,title:'<img src=x onerror="window.UNTRUSTED=true">',body:'Review fixture',url:'https://github.com/fixture/repo/pull/42',state:'OPEN',draft:false,base:'main',head:'bruhs/fixture',head_oid:'a'.repeat(40),base_oid:'b'.repeat(40),updated_at:'2026-11-13T00:00:00Z',mergeable:'MERGEABLE',review_decision:null,checks:[{name:'unit',state:'COMPLETED',conclusion:'SUCCESS',url:null}],comments:[],reviews:[],files:['src/example.rs']};
const server = createServer(async (req, res) => {
  const chunks = [];for await(const chunk of req) chunks.push(chunk);
  const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : undefined;
  calls.push({url:req.url,method:req.method,body});
  if(req.url.startsWith('/api/delivery/ui')){res.setHeader('content-type','text/html');res.end(html);return;}
  if(req.url==='/api/workbench/ui'||req.url==='/api/coordination/ui'){
    res.setHeader('content-type','text/html');
    res.end(req.url==='/api/workbench/ui'?workbenchHtml:coordinationHtml);
    return;
  }
  if(req.url.startsWith('/fonts/')){res.statusCode=404;res.end();return;}
  let response;
  if(req.url==='/__fixture')response={calls,proposals,receipts,watches,trackers};
  else if(req.url==='/api/coordination')response={tasks:[],specs:[],guest_proposals:[],wake_subscriptions:[],wakes:[]};
  else if(req.url==='/api/coordination/board')response={cards:[],refreshing:false,link_error:null};
  else if(req.url==='/api/reactive')response={sources:[{id:'fixture-source',name:'PR review agent',kind:{type:'custom'}}]};
  else if(req.url==='/api/delivery')response={actions:receipts,watches,events:[]};
  else if(req.url==='/api/delivery/preferences/read')response={merge_method:'squash',follow_templates:false};
  else if(req.url==='/api/delivery/review-guide' && captureGuideRequest){
    const capture = captureGuideRequest;
    captureGuideRequest = null;
    const result = await new Promise(reply => capture({input:body, reply}));
    res.statusCode = result.status;
    response = result.payload;
  }
  else if(req.url==='/api/delivery/review-guide')response={guide:cachedGuide};
  else if(req.url==='/api/delivery/write-draft' && captureWriterRequest){
    const capture = captureWriterRequest;
    captureWriterRequest = null;
    const result = await new Promise(reply => capture({input:body, reply}));
    res.statusCode = result.status;
    response = result.payload;
  }
  else if(req.url==='/api/delivery/read'){
    switch(body.query.type){
      case 'pulls':response={type:'pulls',pulls:[pull],stacks:[]};break;
      case 'pull':response={type:'pull',pull,review_guides_enabled:guidesEnabled};break;
      case 'diff':response={type:'diff',patch:'--- a/src/example.rs\n+++ b/src/example.rs\n@@ -1 +1 @@\n-old\n+new'};break;
      case 'line_comments':response={type:'line_comments',comments:[]};break;
      case 'tracker_issue':response={type:'issue',issue:{number:42,title:'ENG-42: Tracker fixture',body:'Tracker issue context',url:'https://linear.app/team/issue/ENG-42/fixture'}};break;
      case 'issue':response={type:'issue',issue:{number:42,title:'Fix sample',body:'Issue context',url:'https://github.com/fixture/repo/issues/42'}};break;
      case 'scripts':response={type:'scripts',scripts:{setup:[],run:[],teardown:[]}};break;
      default:throw new Error('Unknown query');
    }
  }else if(req.url==='/api/delivery/trackers/read'){response=trackers;}else if(req.url==='/api/delivery/trackers'){trackers=trackers.filter(setting=>setting.config.provider!==body.config.provider);trackers.push({config:body.config,credential_saved:!!body.token});response=trackers;}else if(req.url==='/api/delivery/prepare'){
    response={id:'action-'+proposals.length,path:body.path,action:body.action,commands:[{program:'gh',args:['pr','comment','42','--body-file','-'],stdin:body.action.body||'reviewed'}],expected_head:'a'.repeat(40),expected_branch:'bruhs/fixture',worktree_fingerprint:null,timeout_seconds:body.timeout_seconds,created_at_ms:1,workspace_path:null,issue:null};
    if(['rebase_stack','merge_stack'].includes(body.action.type)){
      const rebase=body.action.type==='rebase_stack';response.commands=[];response.expected_branch='main';
      response.stack={repository:'fixture/repo',base:'main',base_oid:'b'.repeat(40),local_base_oid:rebase?'c'.repeat(40):null,temporary_worktree:rebase?'/fixture/private/stack-run/worktree':null,layers:[{number:41,head:'feature/one',base:'main',head_oid:'d'.repeat(40),base_oid:'b'.repeat(40),local_oid:rebase?'e'.repeat(40):null,replay_after_oid:rebase?'b'.repeat(40):null},{number:42,head:'feature/two',base:'feature/one',head_oid:'f'.repeat(40),base_oid:'d'.repeat(40),local_oid:rebase?'f'.repeat(40):null,replay_after_oid:rebase?'d'.repeat(40):null}]};
    }
    response.forge=body.forge||'auto';response.repository={forge:body.forge==='git_lab'?'git_lab':'git_hub',host:body.forge==='git_lab'?'gitlab.com':'github.com',project:'fixture/repo',remote_url:'git@github.com:fixture/repo.git'};
    if(body.action.type==='start_tracker_issue'){response.commands=[{program:'git',args:['worktree','add','-b',body.action.branch,'/fixture/issue-worktree',body.action.base],stdin:null}];response.workspace_path='/fixture/issue-worktree';response.issue={number:42,title:'ENG-42: Tracker fixture',body:'Tracker issue context',url:'https://linear.app/team/issue/ENG-42/fixture'};}
    if(body.action.type==='merge_native_stack'){response.commands=[];response.native_stack={repository:'fixture/repo',stack_number:7,base:'main',base_oid:'b'.repeat(40),merge_through:42,layers:[{number:42,head:'bruhs/fixture',base:'main',head_oid:'a'.repeat(40),base_oid:'b'.repeat(40)}]};}
    proposals.push(response);receipts.unshift({prepared:response,status:'prepared',completed_commands:0,output:'',error:null,updated_at_ms:1});
  }else if(req.url==='/api/delivery/execute'){
    const receipt=receipts.find(receipt=>receipt.prepared.id===body.id);assert.equal(receipt.status,'prepared');receipt.status='completed';receipt.completed_commands=receipt.prepared.stack?2:1;receipt.output='Delivered';response=receipt;
  }else if(req.url==='/api/delivery/execute-native-stack'){
    assert.equal(body.acknowledge_server_heads,true);const receipt=receipts.find(receipt=>receipt.prepared.id===body.id);assert.equal(receipt.status,'prepared');receipt.status='interrupted';receipt.native_operation={state:'pending',operation_id:'3a31b937-d5c4-4ddb-a87c-e534603282f2',message:'Fixture pending'};response=receipt;
  }else if(req.url==='/api/delivery/native-stack-status'){
    const receipt=receipts.find(receipt=>receipt.prepared.id===body.id);receipt.status='completed';receipt.completed_commands=1;receipt.native_operation.state='merged';response=receipt;
  }else if(req.url==='/api/delivery/cancel'){
    receipts.find(receipt=>receipt.prepared.id===body.id).status='cancelled';response={actions:receipts,watches,events:[]};
  }else if(req.url==='/api/delivery/watches'){
    response={id:'watch-1',request:body,enabled:true,next_poll_at_ms:1,previous:pull,owner_login:'fixture',error:null};watches.push(response);
  }else if(req.url==='/api/delivery/watches/watch-1'&&req.method==='DELETE'){
    watches=[];response={actions:receipts,watches,events:[]};
  }else{res.statusCode=404;response={error:'Unknown fixture endpoint'};}
  res.setHeader('content-type','application/json');res.end(JSON.stringify(response));
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const origin='http://127.0.0.1:'+server.address().port;
if (process.argv.includes('--serve')) {
  console.log(JSON.stringify({origin,ui:origin+'/api/delivery/ui?path=%2Ffixture%2Frepo'}));
  await new Promise(resolve=>process.on('SIGTERM',resolve));
  server.closeAllConnections();
  await new Promise(resolve=>server.close(resolve));
  process.exit(0);
}
const { chromium } = createRequire(import.meta.url)(process.env.BRUHS_PLAYWRIGHT_PATH || 'playwright');
const browser=await chromium.launch({headless:true});
const guideFixture = {
 head_oid: pull.head_oid, base_oid: pull.base_oid, model:'fixture/model',
 stops:[
  {title:'Validate the request',summary:'The delivery host checks the reviewed revision before applying a change.',
   attention:{kind:'contract',note:'Both refs must match the revision shown to the reviewer.'},
   files:[{path:'tests/review.rs',note:'Covers mismatched revisions.'}],
   excerpts:[{path:'src/review.rs',side:'new',start:8,end:9,caption:'Validate both commit IDs.',
    code:'if current != reviewed {\n    return Err(Changed);'}],
   questions:['Which revision should a retry use?'],
   notes:[{path:'src/review.rs',line:8,title:'Comparison order',body:'Check both refs before preparing the action.'}]},
  {title:'Remove the fallback',summary:'The checkout no longer accepts an implicit revision.',attention:null,files:[],
   excerpts:[{path:'src/legacy.rs',side:'old',start:5,end:5,caption:'The removed fallback.',code:'<img src=x onerror="window.UNTRUSTED=true">'}],
   questions:[], notes:[]}
 ]
};
async function checkReviewGuide(page, width) {
 const id = name => page.locator('#'+name);
 const tab = name => page.getByRole('tab',{name,exact:true});
 const btn = name => page.getByRole('button',{name,exact:true});
 const heading = name => page.getByRole('heading',{name,exact:true});
 const scope = {path:'/fixture/repo',forge:'auto',number:42,head_oid:pull.head_oid,base_oid:pull.base_oid};
 const count = () => calls.filter(call=>call.url==='/api/delivery/review-guide').length;
 const before = count();
 const readPull = async() => {
  await Promise.all([
   page.waitForResponse(res=>res.url()===origin+'/api/delivery/read'),
   btn('Read PR').click()
  ]);
  await page.waitForFunction(()=>!document.getElementById('delivery').hasAttribute('aria-busy'));
  await page.getByText('bruhs/fixture → main · OPEN · MERGEABLE',{exact:true}).waitFor();
 };
 async function capture(action, flags) {
  const captured = new Promise(resolve=>{captureGuideRequest=resolve;});
  await Promise.all([
   page.waitForRequest(req=>req.url()===origin+'/api/delivery/review-guide'&&req.method()==='POST'),
   action()
  ]);
  const request = await captured;
  assert.deepEqual(request.input, {...scope,...flags});
  return async (payload, status=200) => {
   const arrived=page.waitForResponse(res=>res.url()===origin+'/api/delivery/review-guide');
   request.reply({payload,status});
   await (await arrived).finished();
   await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  };
 }
 await id('number').fill('42');
 for (const flag of [undefined,false]) {
  guidesEnabled=flag;
  await readPull();
  assert.equal(await tab('Guide').count(),0);
  assert.equal(await count(),before);
 }
 guidesEnabled=true;
 await readPull();
 assert.equal(await tab('Overview').getAttribute('aria-selected'),'true');
 assert.equal(await count(),before);
 await tab('Overview').focus();
 const readFlags={generate:false,refresh:false},firstWrite={generate:true,refresh:false},rewrite={generate:true,refresh:true};
 const lookup=await capture(()=>tab('Overview').press('ArrowRight'),readFlags);
 await page.getByText('Loading review guide…',{exact:true}).waitFor();
 await lookup({error:'Guide index unavailable.'},503);
 await page.getByText('Guide index unavailable.',{exact:true}).waitFor();
 const retry=await capture(()=>btn('Retry loading guide').click(),readFlags);
 await retry({guide:null});
 await heading('No review guide yet').waitFor();
 const writing=await capture(()=>btn('Write guide').click(),firstWrite);
 await page.getByText('Writing a review guide…',{exact:true}).waitFor();
 assert.ok(await btn('Write guide').isDisabled());
 await tab('Overview').click();
 assert.ok(await page.getByText('Review fixture',{exact:true}).isVisible());
 await writing({guide:guideFixture});
 await tab('Guide').click();
 await heading('Validate the request').waitFor();
 for(const value of ['src/review.rs:8–9 · After','Validate both commit IDs.','Both refs must match the revision shown to the reviewer.',
    'Which revision should a retry use?','Comparison order','tests/review.rs','Covers mismatched revisions.']){
  await page.getByText(value,{exact:true}).waitFor();
 }
 assert.equal(await id('pull-guide-panel').locator('pre').textContent(),guideFixture.stops[0].excerpts[0].code);
 assert.deepEqual(await id('pull-guide-panel').locator('.guide-code-line').evaluateAll(rows=>rows.map(row=>row.dataset.line)),['8','9']);
 await page.getByText('Technical details',{exact:true}).click();
 await page.getByText('Model: fixture/model',{exact:true}).waitFor();
 await page.getByText('Technical details',{exact:true}).click();
 assert.ok(await btn('Previous stop').isDisabled());
 if(width<850) {
  await page.getByRole('combobox',{name:'Review stops'}).selectOption('1');
 } else {
  await page.getByRole('navigation',{name:'Review stops'}).getByRole('button',{name:'2. Remove the fallback'}).click();
 }
 await heading('Remove the fallback').waitFor();
 assert.ok(await heading('Remove the fallback').evaluate(el=>el===document.activeElement));
 assert.ok(await btn('Next stop').isDisabled());
 await page.getByText('src/legacy.rs:5 · Before',{exact:true}).waitFor();
 assert.equal(await id('pull-guide-panel').locator('img').count(),0);
 await heading('Remove the fallback').press('k');
 await heading('Validate the request').waitFor();
 await heading('Validate the request').press('j');
 await heading('Remove the fallback').waitFor();
 const beforeFinish=proposals.length;
 await btn('Finish review').click();
 assert.equal(await id('action').inputValue(),'review');
 assert.equal(await id('head-oid').inputValue(),pull.head_oid);
 assert.ok(await id('note-body').evaluate(el=>el===document.activeElement));
 assert.equal(proposals.length,beforeFinish);
 await btn('Previous stop').click();
 await heading('Validate the request').waitFor();
 await btn('Next stop').click();
 const rewriteError=await capture(()=>btn('Rewrite guide').click(),rewrite);
 await page.getByText('Rewriting guide…',{exact:true}).waitFor();
 assert.ok(await heading('Remove the fallback').isVisible());
 await rewriteError({error:'Guide writer temporarily unavailable.'},503);
 assert.ok(await heading('Remove the fallback').isVisible());
 await page.getByText('Guide writer temporarily unavailable.',{exact:true}).waitFor();
 const updated = {...guideFixture,stops:[{...guideFixture.stops[0],title:'Revised stop'}]};
 const rewriteOK=await capture(()=>btn('Rewrite guide').click(),rewrite);
 await rewriteOK({guide:updated});
 await heading('Revised stop').waitFor();
 assert.ok(await btn('Next stop').isDisabled());
 // Revisiting the same path and PR must not revive a reply to the previous view.
 const late=await capture(()=>btn('Rewrite guide').click(),rewrite);
 await id('repo').fill('/fixture/other');
 await id('repo').fill(scope.path);
 await readPull();
 const current=await capture(()=>tab('Guide').click(),readFlags);
 await current({guide:guideFixture});
 await late({guide:updated});
 await heading('Validate the request').waitFor();
 assert.equal(await heading('Revised stop').count(),0);
 // Edits to the PR number invalidate in-flight generation even if the text is restored.
 const outdated=await capture(()=>btn('Rewrite guide').click(),rewrite);
 await id('number').fill('41');
 await id('number').fill('42');
 await outdated({error:'Old PR error'},503);
 assert.ok(await id('read-result').isHidden());
 await readPull();
 const mismatched=await capture(()=>tab('Guide').click(),readFlags);
 await mismatched({guide:{...guideFixture,head_oid:'e'.repeat(40)}});
 await page.getByText('This guide does not match the reviewed change. Read the pull request again.',{exact:true}).waitFor();
 assert.equal(await heading('Validate the request').count(),0);
 const valid=await capture(()=>btn('Retry loading guide').click(),readFlags);
 await valid({guide:guideFixture});
 await heading('Validate the request').waitFor();
 assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
 guidesEnabled=undefined;
}

async function checkGuideFileLinks(page) {
 const originalUrl=pull.url, originalHead=pull.head_oid;
 const base=pull.base_oid, diffBase='c'.repeat(64);
 const stop={...guideFixture.stops[0],
  excerpts:[
   {...guideFixture.stops[0].excerpts[0],path:'src/new file#.rs'},
   {...guideFixture.stops[1].excerpts[0],path:'src/renamed.rs',source_path:'old/renamed %?.rs'}
  ],
  files:[{path:'a/b c.md',note:'New documentation.'},
   {path:'deleted.md',source_path:'prior/deleted.md',source_side:'old',note:'Removed documentation.'},
   {path:'../invalid',note:'Unusable path.'}],
  notes:[{path:'src/new file#.rs',line:8,title:'Revision note',body:'Read the reviewed line.'}]
 };
 const guidePanel=page.getByRole('tabpanel',{name:'Guide'});
 async function openGuide(url,extra={}) {
  pull.url=url;
  cachedGuide={...guideFixture,head_oid:pull.head_oid,stops:[stop],...extra};
  await Promise.all([page.waitForResponse(r=>r.url()===origin+'/api/delivery/read'),page.getByRole('button',{name:'Read PR',exact:true}).click()]);
  await page.waitForFunction(()=>!document.getElementById('delivery').hasAttribute('aria-busy'));
  await page.getByRole('tab',{name:'Guide'}).click();
  await guidePanel.getByRole('heading',{name:stop.title,exact:true}).waitFor();
 }
 async function checkLink(label,url) {
  const link=guidePanel.getByRole('link',{name:label,exact:true});
  assert.equal(await link.getAttribute('href'),url);
  assert.equal(await link.getAttribute('target'),'_blank');
  assert.equal(await link.getAttribute('rel'),'noopener noreferrer');
 }
 guidesEnabled=true;
 try{
  for (const [url,blob,end] of [
   ['https://github.com/fixture/repo/pull/42?view=all#discussion','https://github.com/fixture/repo/blob/','L9'],
   ['http://forge.invalid:8080/group/subgroup/project/-/merge_requests/42','http://forge.invalid:8080/group/subgroup/project/-/blob/','9']
  ]) {
   await openGuide(url,{diff_base_oid:diffBase});
   await checkLink('src/new file#.rs:8–9',blob+originalHead+'/src/new%20file%23.rs#L8-'+end);
   await checkLink('old/renamed %?.rs:5',blob+diffBase+'/old/renamed%20%25%3F.rs#L5');
   await checkLink('src/new file#.rs:8',blob+originalHead+'/src/new%20file%23.rs#L8');
   await checkLink('a/b c.md',blob+originalHead+'/a/b%20c.md');
   await checkLink('prior/deleted.md',blob+diffBase+'/prior/deleted.md');
   assert.equal(await guidePanel.getByRole('link',{name:'../invalid',exact:true}).count(),0);
  }
  await openGuide(originalUrl);
  await checkLink('old/renamed %?.rs:5','https://github.com/fixture/repo/blob/'+base+'/old/renamed%20%25%3F.rs#L5');
  for (const url of ['file:///fixture/repo/pull/42','https://name:token@forge.invalid/fixture/repo/pull/42','https://forge.invalid/fixture/repo/pull/43']) {
   await openGuide(url);
   assert.equal(await guidePanel.getByRole('link').count(),0,url);
   assert.ok(await guidePanel.getByText('a/b c.md',{exact:true}).isVisible());
  }
  pull.head_oid='branch';
  await openGuide(originalUrl,{diff_base_oid:'invalid'});
  assert.equal(await guidePanel.getByRole('link').count(),0,'Invalid OIDs remain readable without links');
 }finally{
  pull.url=originalUrl;pull.head_oid=originalHead;cachedGuide=null;guidesEnabled=undefined;
 }
}

async function checkGuideComposer(page) {
 guidesEnabled=true;
 cachedGuide=guideFixture;
 await page.goto(origin+'/api/workbench/ui#delivery');
 const panel=page.frameLocator('#panel');
 await panel.getByLabel('Repository folder',{exact:true}).fill('/fixture/repo');
 await panel.getByLabel('Issue or pull request number',{exact:true}).fill('42');
 await panel.getByRole('button',{name:'Read PR',exact:true}).click();
 await panel.getByRole('tab',{name:'Guide'}).click();
 await panel.getByRole('button',{name:'Ask about this stop',exact:true}).waitFor();
 const before=calls.filter(call=>call.url==='/api/coordination/commands').length;
 await panel.getByRole('button',{name:'Which revision should a retry use?',exact:true}).click();
 const prompt=panel.locator('#prompt');
 await prompt.waitFor({state:'visible'});
 assert.equal(await panel.locator('#title').inputValue(),'Review PR #42');
 assert.equal(await panel.locator('#project').inputValue(),'/fixture/repo');
 const draft=await prompt.inputValue();
 for (const piece of ['Which revision should a retry use?',pull.head_oid,pull.base_oid,'src/review.rs','Validate the request']) {
  assert.ok(draft.includes(piece),piece);
 }
 assert.ok(await prompt.evaluate(el=>document.activeElement===el));
 assert.equal(calls.filter(call=>call.url==='/api/coordination/commands').length,before);
 cachedGuide=null;
 guidesEnabled=undefined;
}

async function checkWriterDrafts(page) {
 const field = id => page.locator('#' + id);
 const action = field('action'), commit = field('commit-message'), title = field('title'), body = field('body');
 const repo = '/fixture/repo', head = 'bruhs/issue-42', files = ['src/example.rs', 'docs/notes.md'];
 const writesBefore = calls.filter(call => call.url === '/api/delivery/execute').length;
 const previewsBefore = proposals.length;
 // Capture and settle each individual HTTP response independently of wall-clock timing.
 async function start(kind) {
  const received = new Promise(resolve => { captureWriterRequest = resolve; });
  const name = kind === 'commit' ? 'Generate commit message' : 'Generate PR title and description';
  const control = page.getByRole('button', {name, exact:true});
  await Promise.all([
   page.waitForRequest(request => request.url() === origin + '/api/delivery/write-draft' && request.method() === 'POST'),
   control.click()
  ]);
  const request = await received;
  assert.ok(await control.isDisabled());
  return {
   input: request.input,
   async reply(payload, status = 200) {
    const response = page.waitForResponse(response => response.url() === origin + '/api/delivery/write-draft');
    request.reply({payload, status});
    await (await response).finished();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
   }
  };
 }
 assert.ok(await field('write-pull').isHidden());
 assert.ok(await field('write-commit').isHidden());
 await action.selectOption('create_pull');
 await field('head').fill('');
 await field('write-pull').click();
 assert.match(await field('write-pull-status').textContent(), /base and head branches/i);
 await field('head').fill(head);
 const pullDraft = await start('pull');
 assert.deepEqual(pullDraft.input, {path:repo, draft:{kind:'pull', base:'main', head, files:[]}});
 await pullDraft.reply({title:'Draft PR', body:'Generated description'});
 assert.equal(await title.inputValue(), 'Draft PR');
 assert.equal(await body.inputValue(), 'Generated description');

 await action.selectOption('commit_push_pull');
 await field('write-commit').click();
 assert.match(await field('write-commit-status').textContent(), /select the files/i);
 await field('files').fill(files.join('\n'));
 const commitDraft = await start('commit');
 const combinedDraft = await start('pull');
 assert.deepEqual(commitDraft.input, {path:repo, draft:{kind:'commit', files}});
 assert.deepEqual(combinedDraft.input, {path:repo, draft:{kind:'pull', base:'main', head, files}});
 await combinedDraft.reply({title:'Combined PR', body:'Selected file changes'});
 await commitDraft.reply({title:'Describe selected changes', body:'First paragraph.\n\nSecond paragraph.'});
 assert.equal(await title.inputValue(), 'Combined PR');
 assert.equal(await body.inputValue(), 'Selected file changes');
 assert.equal(await commit.inputValue(), 'Describe selected changes\n\nFirst paragraph.\n\nSecond paragraph.');
 const summaryOnly = await start('commit');
 await summaryOnly.reply({title:'One-line commit', body:''});
 assert.equal(await commit.inputValue(), 'One-line commit');
 const writerError = await start('commit');
 await writerError.reply({error:'Writer model is unavailable.'}, 503);
 assert.equal(await field('write-commit-status').textContent(), 'Writer model is unavailable.');
 assert.equal(await commit.inputValue(), 'One-line commit');
 const invalid = await start('pull');
 await invalid.reply({title:'Incomplete'});
 assert.match(await field('write-pull-status').textContent(), /incomplete draft/i);
 assert.equal(await body.inputValue(), 'Selected file changes');

 const staleCommit = await start('commit');
 await commit.fill('Edited commit message\n\nBody kept by the author.');
 await staleCommit.reply({title:'Stale', body:'Old commit'});
 assert.equal(await commit.inputValue(), 'Edited commit message\n\nBody kept by the author.');
 for (const [editor, edit] of [[title,'Edited PR title'], [body,'Edited PR description']]) {
  const late = await start('pull');
  await editor.fill(edit);
  await late.reply({title:'Stale', body:'Old description'});
  assert.equal(await editor.inputValue(), edit);
 }
 // Restoring the same values still invalidates the original request.
 for (const [id, different] of [['repo','/fixture/other'], ['action','create_pull'],
                                ['base','release'], ['head','another-branch'], ['files','other.rs']]) {
  const changed = field(id), original = await changed.inputValue();
  const late = await start('pull');
  if (id === 'action') {
   await changed.selectOption(different);
   await changed.selectOption(original);
  } else {
   await changed.fill(different);
   await changed.fill(original);
  }
  await late.reply({title:'Stale ' + id, body:'Stale ' + id});
  assert.equal(await title.inputValue(), 'Edited PR title', id);
  assert.equal(await body.inputValue(), 'Edited PR description', id);
  assert.equal(await field('write-pull-status').textContent(), '', id);
 }
 const otherRepo = await start('pull');
 await field('repo').fill('/fixture/other');
 await otherRepo.reply({title:'Wrong repository', body:'Wrong repository'});
 assert.equal(await title.inputValue(), '');
 assert.equal(await commit.inputValue(), '');
 assert.equal(await body.inputValue(), '');
 await field('repo').fill(repo);
 const editedBack = await start('pull');
 const originalTitle = await title.inputValue();
 await title.fill('Temporary edit');
 await title.fill(originalTitle);
 await editedBack.reply({title:'Stale after undo', body:'Stale after undo'});
 assert.equal(await title.inputValue(), originalTitle);
 assert.equal(await body.inputValue(), 'Edited PR description');
 const superseded = await start('pull');
 await body.fill('Waiting for a newer draft');
 const newer = await start('pull');
 await newer.reply({title:'Current title', body:'Current body'});
 await superseded.reply({error:'Old error'}, 503);
 assert.equal(await title.inputValue(), 'Current title');
 assert.equal(await body.inputValue(), 'Current body');
 assert.equal(await field('write-pull-status').textContent(), 'Draft ready to edit.');
 assert.equal(proposals.length, previewsBefore);
 await title.fill('Ready for review');
 await body.fill('Reviewed PR body');
 await page.getByRole('button',{name:'Preview action',exact:true}).click();
 await page.getByRole('button',{name:'Review independent checks',exact:true}).waitFor();
 assert.equal(proposals.length, previewsBefore + 1);
 const proposal = proposals.at(-1).action;
 assert.equal(proposal.type, 'commit_push_pull');
 assert.equal(proposal.message, 'Edited commit message\n\nBody kept by the author.');
 assert.equal(proposal.title, 'Ready for review');
 assert.equal(proposal.body, 'Reviewed PR body');
 assert.equal(calls.filter(call => call.url === '/api/delivery/execute').length, writesBefore);
}
const failures=[];
try{
 for(const width of [1280,375]){
  proposals=[];receipts=[];watches=[];calls.length=0;
  const page=await browser.newPage({viewport:{width,height:900}});page.on('pageerror',failure=>failures.push(failure.message));
  await page.goto(origin+'/api/delivery/ui?path=%2Ffixture%2Frepo');
  await page.getByRole('button',{name:'Pull requests',exact:true}).click();
  await page.getByRole('button',{name:'#42 <img src=x onerror="window.UNTRUSTED=true">',exact:true}).click();
  await page.getByLabel('Reviewed head commit, loaded by Read PR').waitFor({state:'attached'});
  assert.equal(await page.evaluate(()=>window.UNTRUSTED),undefined);
  assert.equal(await page.locator('#read-result img').count(),0);
  await page.locator('#action-details > summary').click();
  await page.getByLabel('Action',{exact:true}).selectOption('review');
  await page.getByLabel('Comment or review message').fill('first line\n$(touch NEVER) `touch NEVER`\nlast line');
  await page.getByText('Add a line comment',{exact:true}).click();
  await page.getByLabel('Relative file path',{exact:true}).fill('src/example.rs');
  await page.getByLabel('Line number',{exact:true}).fill('3');
  await page.getByLabel('Line comment',{exact:true}).fill('Please cover this branch.');
  await page.getByRole('button',{name:'Attach line comment',exact:true}).click();
  await page.getByLabel('Execution timeout, seconds').fill('30');
  await page.getByRole('button',{name:'Preview action',exact:true}).click();
  await page.getByRole('button',{name:'Run reviewed action',exact:true}).waitFor();
  assert.equal(calls.filter(call=>call.url==='/api/delivery/execute').length,0);
  assert.equal(proposals[0].action.comments[0].line,3);
  assert.equal(proposals[0].action.comments[0].side,'RIGHT');
  assert.match(proposals[0].action.body,/\$\(touch NEVER\)/);
  await page.getByRole('button',{name:'Run reviewed action',exact:true}).click();
  await page.getByText('Completed · 1/1 steps · bruhs/fixture',{exact:true}).waitFor();
  assert.equal(calls.filter(call=>call.url==='/api/delivery/execute').length,1);
  await page.getByText('Watch this pull request',{exact:true}).click();
  await page.getByLabel('Poll interval, seconds').fill('30');
  await page.getByRole('button',{name:'Load agent event sources',exact:true}).click();
  await page.getByLabel('Agent event source',{exact:true}).selectOption('fixture-source');
  await page.getByRole('button',{name:'Start or update watch',exact:true}).click();
  await page.getByText('#42 · Watching · every 30 seconds',{exact:true}).waitFor();
  assert.equal(watches[0].request.reactive_source_id,'fixture-source');
  await page.getByRole('button',{name:'Remove watch',exact:true}).click();
  await page.getByText('No watched pull requests.',{exact:true}).waitFor();
  await page.getByRole('button',{name:'Read issue',exact:true}).click();
  await page.getByRole('link',{name:'#42 Fix sample',exact:true}).waitFor();
  assert.equal(await page.locator('#head').inputValue(),'bruhs/issue-42');
  await checkWriterDrafts(page);
  await checkReviewGuide(page,width);
  await checkGuideFileLinks(page);
  await checkGuideComposer(page);
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth), 'No horizontal overflow at '+width);
  await page.close();
 }
 assert.deepEqual(failures,[]);
 console.log(JSON.stringify({passed:true,widths:[1280,375],cases:['untrusted title stays text','line review payload','multiline input preserved','preview does not execute','explicit execution once','watch add/remove','issue-to-worktree branch','editable commit and PR drafts','writer errors preserve edits','stale and out-of-order writer responses','writer requires explicit preview and execution','guide opt-in, cached and explicit generation','guide stop navigation and excerpts','guide refresh, failure and stale replies','revision-pinned GitHub and GitLab file links','guide keyboard navigation, review and composer handoffs','no horizontal overflow','no JS errors']}));
}finally{await browser.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
