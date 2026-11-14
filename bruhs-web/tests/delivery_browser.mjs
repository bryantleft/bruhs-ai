// Offline UI acceptance: no GitHub credentials, model calls, or repository writes.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
const html = await readFile(new URL('../../bruhs-server/src/routes/delivery_ui.html', import.meta.url));
const calls = [];
let proposals = [], receipts = [], watches = [], trackers = [];
const pull = {number:42,title:'<img src=x onerror="window.UNTRUSTED=true">',body:'Review fixture',url:'https://github.com/fixture/repo/pull/42',state:'OPEN',draft:false,base:'main',head:'bruhs/fixture',head_oid:'a'.repeat(40),updated_at:'2026-11-13T00:00:00Z',mergeable:'MERGEABLE',review_decision:null,checks:[{name:'unit',state:'COMPLETED',conclusion:'SUCCESS',url:null}],comments:[],reviews:[],files:['src/example.rs']};
const server = createServer(async (req, res) => {
  const chunks = [];for await(const chunk of req) chunks.push(chunk);
  const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : undefined;
  calls.push({url:req.url,method:req.method,body});
  if(req.url.startsWith('/api/delivery/ui')){res.setHeader('content-type','text/html');res.end(html);return;}
  if(req.url.startsWith('/fonts/')){res.statusCode=404;res.end();return;}
  let response;
  if(req.url==='/__fixture')response={calls,proposals,receipts,watches,trackers};
  else if(req.url==='/api/reactive')response={sources:[{id:'fixture-source',name:'PR review agent',kind:{type:'custom'}}]};
  else if(req.url==='/api/delivery')response={actions:receipts,watches,events:[]};
  else if(req.url==='/api/delivery/read'){
    switch(body.query.type){
      case 'pulls':response={type:'pulls',pulls:[pull],stacks:[]};break;
      case 'pull':response={type:'pull',pull};break;
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
  await page.getByLabel('Description or review message').fill('first line\n$(touch NEVER) `touch NEVER`\nlast line');
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
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth), 'No horizontal overflow at '+width);
  await page.close();
 }
 assert.deepEqual(failures,[]);
 console.log(JSON.stringify({passed:true,widths:[1280,375],cases:['untrusted title stays text','line review payload','multiline input preserved','preview does not execute','explicit execution once','watch add/remove','issue-to-worktree branch','no horizontal overflow','no JS errors']}));
}finally{await browser.close();server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
