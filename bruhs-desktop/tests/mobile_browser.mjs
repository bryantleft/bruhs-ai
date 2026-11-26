import {readFile,writeFile,mkdir,access} from "node:fs/promises";
import {join,dirname} from "node:path";
import {createRequire} from "node:module";
import {createHash,X509Certificate} from "node:crypto";
import assert from "node:assert/strict";
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.BRUHS_PLAYWRIGHT_MODULE||"playwright");
const fixture=JSON.parse(await readFile(process.argv[2],"utf8"));
const output=process.env.BRUHS_MOBILE_QA_DIR||join(dirname(process.argv[2]),"browser-results");
await mkdir(output,{recursive:true});
const checks=[],failures=[],commands=[];let snapshots=0;
const cleanup=[];
const stage=async name=>writeFile(join(output,"stage.json"),JSON.stringify({stage:name,checks:checks.length,at:new Date().toISOString()}));
async function deadline(promise,label){let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(`Timed out: ${label}`)),15000);})]);}finally{clearTimeout(timer);}}
const safeError=error=>String(error.message||error).replace(/Bearer [A-Za-z0-9_-]+/g,"Bearer [redacted]").replace(/#pair=[A-Za-z0-9_-]+/g,"#pair=[redacted]");
function check(value,name){assert(value,name);checks.push(name);}
const spki=createHash("sha256").update(new X509Certificate(await readFile(fixture.certificate)).publicKey.export({type:"spki",format:"der"})).digest("base64");
const browser=await chromium.launch({headless:true,executablePath:process.env.BRUHS_CHROME,args:[`--ignore-certificate-errors-spki-list=${spki}`]});
const waitFile=async name=>{const end=Date.now()+15000;while(Date.now()<end){try{await access(join(fixture.control,name));return JSON.parse(await readFile(join(fixture.control,name),"utf8"));}catch{await new Promise(r=>setTimeout(r,100));}}throw new Error(`Native fixture did not answer ${name}`);};
const phase=name=>writeFile(join(fixture.control,name),"{}");
function inspect(page){
  page.setDefaultTimeout(15000);
  page.on("pageerror",error=>failures.push(error.message));
  page.on("request",request=>{if(request.url().endsWith("/api/mobile/command")){try{commands.push(JSON.parse(request.postData()));}catch{}}});
  page.on("response",async response=>{
    if(!response.url().includes("/api/mobile/")||response.url().endsWith("/events"))return;
    try{const bytes=await response.body();assert(bytes.length<=32768,"bounded API frame");assert(!bytes.includes(Buffer.from("PRIVATE_PHONE_")),"private input/metadata/thinking exclusion");snapshots++;}catch(error){if(!String(error).includes("No resource with given identifier")&&!String(error).includes("Protocol error")&&!String(error).includes("Target page"))failures.push(error.message);}
  });
}
async function home(page){await page.locator("#home").click();await page.getByRole("heading",{name:"Your desktop threads"}).waitFor();}
async function thread(page,name="Native phone thread"){await page.getByRole("button",{name:new RegExp(name)}).click();await page.getByRole("heading",{name,exact:true}).waitFor();await page.getByText("Laptop connected",{exact:true}).waitFor();}
async function send(page,text){await page.locator("#prompt").fill(text);await page.getByRole("button",{name:"Send prompt",exact:true}).click();await page.locator("#prompt-status").filter({hasText:"Accepted into the desktop queue"}).waitFor();}
async function authorized(page,path,body){return page.evaluate(async({path,body})=>{const token=localStorage.getItem("bruhs.mobile.device.v1");const response=await fetch(path,{method:body?"POST":"GET",headers:{Authorization:`Bearer ${token}`,...(body?{"Content-Type":"application/json"}:{})},...(body?{body:JSON.stringify(body)}:{})});return{status:response.status,value:await response.json()};},{path,body});}
try{
  await stage("pair");
  const context=await browser.newContext({viewport:{width:390,height:844},isMobile:true,hasTouch:true});
  const page=await context.newPage();inspect(page);
  await page.goto(fixture.pairing);
  await page.getByRole("heading",{name:"Pair with your laptop"}).waitFor();check(!page.url().includes("#"),"pairing fragment removed before exchange");
  await page.getByLabel("Device name").fill("Touch phone");await page.getByRole("button",{name:"Pair this device"}).click();
  await page.getByRole("heading",{name:"Your desktop threads"}).waitFor();
  await page.getByRole("button",{name:/Native phone thread/}).first().waitFor();
  check(await page.getByRole("button",{name:/Native phone thread/}).count()===1,"actual native project thread list");
  check(!await page.getByText("PRIVATE_NONPROJECT_CHAT_CANARY").count(),"nonproject chats excluded");
  await thread(page);await page.getByText("Actual native live output",{exact:true}).waitFor();
  await stage("approval and drafts");
  check(await page.getByText("A private action needs the laptop",{exact:false}).isVisible(),"credential action remains laptop-only");
  await page.getByRole("button",{name:"Allow this tool",exact:true}).click();
  await page.getByRole("heading",{name:"Tool approval · Read"}).waitFor({state:"detached"});check(true,"exact native approval consumed");
  await page.locator("#prompt").fill("Draft on first thread");await home(page);await thread(page,"Second native thread");
  check(await page.locator("#prompt").inputValue()==="","phone draft does not cross threads");
  await page.locator("#prompt").fill("Draft on second thread");await home(page);await thread(page);
  check(await page.locator("#prompt").inputValue()==="Draft on first thread","phone thread draft restored");
  await send(page,"First phone queued prompt");
  await stage("history and live");
  const first=commands.at(-1);const retry=await authorized(page,"/api/mobile/command",first);
  check(retry.status===200&&retry.value.id===first.id&&retry.value.outcome.status==="accepted","exact HTTP retry returns receipt without another native dispatch");
  await page.getByRole("button",{name:"Earlier messages"}).click();await page.getByText("Completed native message 0",{exact:true}).waitFor();check(true,"older native history pages");
  await page.getByRole("button",{name:"Latest",exact:true}).click();
  await phase("live");await page.getByText(/Live update from actual native ChatView/).waitFor();
  check(await page.locator("#messages img").count()===0,"streamed HTML is inert text");
  check(!await page.evaluate(()=>Boolean(window.MOBILE_XSS)),"inert content did not execute");
  await page.screenshot({path:join(output,"touch-thread.png"),fullPage:true});
  await page.locator("#prompt").fill("Unsent draft during diff review");
  await stage("diff");
  await page.getByRole("button",{name:"Review diff",exact:true}).click();await page.getByRole("heading",{name:"Working tree diff"}).waitFor();
  await page.locator(".text-page").filter({hasText:"+native fixture changed"}).waitFor();check(true,"diff resolves actual native thread worktree");
  await page.getByRole("button",{name:"Next page",exact:true}).click();await page.locator(".text-page").waitFor();check(true,"diff pages retain revision");
  await stage("ambiguous reload");
  await page.getByRole("button",{name:"← Thread",exact:true}).click();await page.getByRole("heading",{name:"Native phone thread",exact:true}).waitFor();
  check(await page.locator("#prompt").inputValue()==="Unsent draft during diff review","phone draft survives a diff round trip");
  // Accept natively, lose the HTTP response, then reload the same tab.
  let serverAccepted,serverFailed,release;const accepted=new Promise((resolve,reject)=>{serverAccepted=resolve;serverFailed=reject;}),held=new Promise(r=>release=r);cleanup.push(()=>release());accepted.catch(()=>{});
  await page.route("**/api/mobile/command",async route=>{try{const response=await route.fetch();const value=await response.json();serverAccepted(value);await held;await route.abort("failed");}catch(error){serverFailed(new Error(safeError(error)));}});
  await page.locator("#prompt").fill("Ambiguous reload prompt");await page.getByRole("button",{name:"Send prompt",exact:true}).click();
  const receipt=await deadline(accepted,"server acceptance before reload");check(receipt.outcome.status==="accepted","ambiguous command committed before response loss");
  const countBeforeReload=commands.length;
  const navigation=page.waitForEvent("framenavigated",{predicate:frame=>frame===page.mainFrame()});const reload=page.reload();await deadline(navigation,"reload navigation");release();await deadline(reload,"reload document");await page.unroute("**/api/mobile/command");
  await page.getByRole("heading",{name:"Your desktop threads"}).waitFor();await thread(page);
  await page.locator("#prompt-status").filter({hasText:"Accepted into the desktop queue"}).waitFor();
  check(commands.length===countBeforeReload,"reload reconciles stored command without replay");
  check(await page.evaluate(()=>sessionStorage.getItem("bruhs.mobile.pending.v1"))===null,"terminal receipt clears tab command storage");
  await stage("two tab sequence recovery");
  const second=await context.newPage();inspect(second);await second.goto(fixture.origin);await thread(second);
  await send(page,"Advance sequence in first tab");
  await second.locator("#prompt").fill("Stale second tab command");await second.getByRole("button",{name:"Send prompt",exact:true}).click();
  await second.locator("#prompt-status").filter({hasText:"delivery cannot be established"}).waitFor();
  await send(second,"Recovered second tab command");check(true,"stale sequence reconciles and unlocks second tab");
  await stage("offline recovery");
  await context.setOffline(true);await page.getByText("Offline · showing last received data",{exact:true}).waitFor();
  check(await page.getByRole("button",{name:"Send prompt",exact:true}).isDisabled(),"offline disables mutation controls");
  check(await page.getByText(/Live update from actual native ChatView/).count()>0,"offline retains marked stale snapshot");
  await context.setOffline(false);await page.getByText("Laptop connected",{exact:true}).waitFor();check(true,"reconnect resnapshots native state");
  await stage("read only compact device");
  await phase("offer-readonly");const readOffer=await waitFile("readonly-offer.json");
  const readContext=await browser.newContext({viewport:{width:640,height:480},hasTouch:true});const reader=await readContext.newPage();inspect(reader);await reader.goto(readOffer.url);
  await reader.getByLabel("Device name").fill("Read only compact phone");await reader.getByRole("button",{name:"Pair this device"}).click();await thread(reader,"Second native thread");
  check(await reader.getByRole("button",{name:"Send prompt",exact:true}).isDisabled(),"read-only phone cannot send prompts");check(await reader.getByRole("button",{name:"Review diff",exact:true}).isDisabled(),"diff is an independent scope");
  check(await reader.getByRole("button",{name:"Allow this tool",exact:true}).isDisabled(),"approval is an independent scope");
  const denied=await authorized(reader,"/api/mobile/command",{id:crypto.randomUUID(),sequence:1,action:{kind:"prompt",thread_id:fixture.first,text:"Not granted"}});check(denied.status===403,"backend rejects read-only phone writes");
  await reader.screenshot({path:join(output,"compact-readonly.png"),fullPage:true});
  for(const p of [page,second,reader])check(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),"no horizontal overflow at tested viewport");
  await stage("revocation and disconnect");
  await phase("revoke");await page.getByRole("heading",{name:"Desktop access ended"}).waitFor();await second.getByRole("heading",{name:"Desktop access ended"}).waitFor();
  check(await page.evaluate(()=>localStorage.getItem("bruhs.mobile.device.v1"))===null,"revocation clears phone token and displayed data");
  check(!await page.getByText(/Live update from actual native ChatView/).count(),"revocation clears stale chat content");
  await reader.getByRole("button",{name:"Disconnect",exact:true}).click();await reader.getByRole("heading",{name:"Desktop access ended"}).waitFor();check(true,"phone disconnect revokes its own device");
  check(snapshots>10,"real bounded HTTPS snapshots observed");check(failures.length===0,`browser errors: ${failures.join("; ")}`);
  await readContext.close();await context.close();
  const result={passed:checks.length,checks,snapshots,commands:commands.map(c=>({sequence:c.sequence,kind:c.action.kind})),viewports:[{width:390,height:844,touch:true},{width:640,height:480,touch:true}],transport:"actual desktop GPUI broker over trusted isolated HTTPS",browser:browser.version()};
  await writeFile(join(output,"validation.json"),JSON.stringify(result,null,2));process.stdout.write(JSON.stringify({passed:result.passed,snapshots:result.snapshots})+"\n");
}catch(error){const message=safeError(error);await writeFile(join(output,"failure.json"),JSON.stringify({passed:checks.length,checks,error:message,failures},null,2));process.stderr.write(message+"\n");process.exitCode=1;}
finally{for(const release of cleanup)release();await browser.close();}
