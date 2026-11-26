"use strict";
// The companion application is deliberately not loaded on this origin.
const app = document.querySelector("#app"), connection = document.querySelector("#connection");
const disconnect = document.querySelector("#disconnect");
const storageKey = "bruhs.mobile.device.v1";
const commandKey = "bruhs.mobile.pending.v1";
let token = "", session = null, online = false, stopped = false, streamController = null;
let view = {kind:"threads",offset:0}, generation = 0, refreshTask = null, refreshAgain = false;
let snapshot = null, pendingCommand = null, sending = false, promptDraft = "", status = "";
let messageSignature = "", permissionSignature = "", listSignature = "";
const drafts = new Map();
const questionDrafts = new Map();
let restoredCommand=false;
const controllers = new Set();
const el = (tag, text, className) => { const n = document.createElement(tag); if (text !== undefined) n.textContent = text; if (className) n.className = className; return n; };
const button = (text, action, className) => { const b = el("button",text,className); b.type="button"; b.addEventListener("click",action); return b; };
const scopes = () => session?.device.scopes || [];
function controls() {
  document.querySelectorAll("[data-mutation]").forEach(n => {n.disabled = n.dataset.blocked === "true" || !online || sending || !!pendingCommand || !scopes().includes(n.dataset.mutation);});
  const retry = document.querySelector("#retry-command"); if (retry) retry.disabled = !online || sending;
  document.querySelectorAll("[data-scope]").forEach(n=>n.disabled=!online||!scopes().includes(n.dataset.scope));
  connection.textContent = !token ? (stopped?"Not paired":"Ready to pair") : online ? "Laptop connected" : "Offline · showing last received data";
  connection.className = online ? "busy" : "muted";
  disconnect.hidden = !token;
}
function notice(text) { let n = document.querySelector("#notice"); if (!n) { n=el("p",undefined,"notice"); n.id="notice"; app.prepend(n); } n.textContent=text; }
function forget(message) {
  stopped=true; online=false; streamController?.abort();
  for (const c of controllers) c.abort(); controllers.clear();
  token=""; session=null; snapshot=null; pendingCommand=null; promptDraft=""; status="";
  drafts.clear();
  questionDrafts.clear();
  try {localStorage.removeItem(storageKey);} catch {}
  try {sessionStorage.removeItem(commandKey);} catch {}
  generation++; app.replaceChildren(el("h1","Desktop access ended"),el("p",message,"muted")); controls();
}
async function api(path, options={}) {
  const {timeout=16000, ...request}=options;
  const controller = new AbortController(); controllers.add(controller);
  const timer = setTimeout(()=>controller.abort(),timeout);
  try {
    const response=await fetch(path,{...request,credentials:"omit",cache:"no-store",redirect:"error",signal:controller.signal,headers:{...(token?{Authorization:`Bearer ${token}`} : {}),...(request.body?{"Content-Type":"application/json"}:{}),...request.headers}});
    const raw=await response.text();
    if (raw.length>32768) throw new Error("The desktop response is too large.");
    let value; try {value=JSON.parse(raw);} catch {throw new Error("The desktop returned an unreadable response.");}
    if (response.status===401 && token) {forget("This device was revoked, expired, or the desktop restarted. Pair again from the laptop.");throw new Error("Pair again from the laptop.");}
    if (!response.ok) {const error=new Error(value.error || `Desktop request failed (${response.status}).`);error.status=response.status;throw error;}
    return value;
  } finally {clearTimeout(timer);controllers.delete(controller);}
}
function setView(next) {
  rememberDraft();
  promptDraft=next.kind==="thread"?(drafts.get(next.id)||""):"";
  view=next; generation++; messageSignature=""; permissionSignature=""; listSignature=""; snapshot=null;
  app.replaceChildren();
  renderView(next);
  controls(); requestRefresh();
}
function rememberDraft(){
  if(view.kind==="thread"){drafts.delete(view.id);drafts.set(view.id,promptDraft);}
  while(drafts.size>50)drafts.delete(drafts.keys().next().value);
}
function renderView(next){
  if (next.kind==="threads") {app.append(el("h1","Your desktop threads"),el("p","Read current and future project chats on this device.","muted"),Object.assign(el("section"),{id:"threads"}));}
  else if (next.kind==="thread") {
    app.append(button("← Threads",()=>setView({kind:"threads",offset:0})),Object.assign(el("h1","Loading thread…"),{id:"thread-title"}));
    const row=el("div",undefined,"row"); row.append(button("Latest",()=>setView({...view,before:undefined})),button("Review diff",()=>openDiff())); row.lastChild.dataset.scope="diff"; app.append(row);
    app.append(Object.assign(el("div"),{id:"permission"}),Object.assign(el("section"),{id:"messages"}),Object.assign(el("div",undefined,"row"),{id:"history"}));
    const label=el("label","Send a prompt to this thread");label.htmlFor="prompt";
    const input=el("textarea");input.id="prompt";input.maxLength=32768;input.value=promptDraft;input.placeholder="Ask the desktop agent…";input.addEventListener("input",()=>promptDraft=input.value);
    const send=button("Send prompt",()=>sendPrompt(),"primary");send.dataset.mutation="prompt";
    const p=el("p",status,"muted");p.id="prompt-status";p.setAttribute("role","status");
    app.append(label,input,el("div",undefined,"row"));app.lastChild.append(send);app.append(p);
    if (!scopes().includes("prompt")) notice("This device has read access. Sending prompts was not granted on the laptop.");
    renderDelivery();
  }
}
function renderDelivery() {
  const p=document.querySelector("#prompt-status"); if (p) p.textContent=status;
  document.querySelector("#retry-command")?.remove();
  if (pendingCommand) {
    const retry=button("Check / retry the same command",()=>sendCommand(pendingCommand));retry.id="retry-command";
    app.append(retry);
  }
  controls();
}
function renderThreads(page) {
  const signature=JSON.stringify(page);if(signature===listSignature)return;listSignature=signature;
  const list=document.querySelector("#threads");if(!list)return;list.replaceChildren();
  if(!page.threads.length)list.append(el("p","No project threads on this desktop page.","muted"));
  for(const thread of page.threads){const b=button("",()=>setView({kind:"thread",id:thread.id} ),"thread");b.append(el("strong",thread.title||"Untitled thread"),el("small",`${thread.project} · ${thread.needs_answer ? "needs answer" : thread.state.replaceAll("_"," ")}`,"muted"));list.append(b);}
  const row=el("div",undefined,"row");if(view.offset)row.append(button("First page",()=>setView({kind:"threads",offset:0})));if(page.next_offset!==null)row.append(button("More threads",()=>setView({kind:"threads",offset:page.next_offset})));list.append(row);
}
function renderThread(value) {
  const firstSnapshot=messageSignature==="",following=document.documentElement.scrollHeight-scrollY-innerHeight<=innerHeight;
  snapshot=value;
  document.querySelector("#thread-title").textContent=value.thread.title||"Untitled thread";
  const permissionKey=JSON.stringify([value.permission,value.question,value.laptop_action_required,value.queued_prompts]);
  if(permissionKey!==permissionSignature){
    permissionSignature=permissionKey;const box=document.querySelector("#permission");box.replaceChildren();
    if(value.laptop_action_required&&!value.question)box.append(el("p","A private action needs the laptop. Credential forms are not shared here.","notice"));
    if(value.queued_prompts)box.append(el("p",`${value.queued_prompts} prompt(s) waiting in the desktop queue.`,"muted"));
    if(value.question)box.append(questionCard(value.thread.id,value.question));
    if(value.permission){const p=value.permission,card=el("section",undefined,"approval");card.append(el("h2",`Tool approval · ${p.tool}`),el("pre",p.input));
      if(!p.can_allow)card.append(el("p","This review is incomplete. Review and allow it on the laptop; you can deny it here.","muted"));
      const row=el("div",undefined,"row");
      for(const [label,decision] of [["Allow this tool","allow"],["Deny","deny"]]){if(decision==="allow"&&!p.can_allow)continue;const b=button(label,()=>permission(p,decision),decision==="allow"?"primary":"danger");b.dataset.mutation="approve";row.append(b);}card.append(row);box.append(card);
      if(!scopes().includes("approve"))box.append(el("p","Approving tools was not granted for this device.","muted"));
    }
  }
  const messagesKey=JSON.stringify(value.messages);
  if(messagesKey!==messageSignature){messageSignature=messagesKey;const list=document.querySelector("#messages");list.replaceChildren();
    for(const m of value.messages){const article=el("article");article.append(el("h2",`${m.role}${m.streaming?" · live":""}`),el("pre",m.text));
      if(m.byte_offset||m.full_bytes>new TextEncoder().encode(m.text).length)article.append(button("Read full message",()=>openMessage(m)));
      list.append(article);
    }
  }
  const history=document.querySelector("#history");history.replaceChildren();if(value.next_before!==null)history.append(button("Earlier messages",()=>setView({...view,before:value.next_before})));
  controls();
  if(view.before===undefined&&!value.permission&&!value.question&&(firstSnapshot||following))window.scrollTo(0,document.documentElement.scrollHeight);
}

function questionFilePath(threadId, request, position, entry, upload) {
  const q = new URLSearchParams({
    thread_id: threadId,
    turn_generation: request.turn_generation,
    request_id: request.request_id,
    review_hash: request.review_hash,
    question: position,
    upload_id: entry.id,
  });
  if (upload) q.set("name", entry.file.name);
  return `/api/mobile/question-file?${q}`;
}

function questionMarkup(markdown, openProject) {
  const tags = {
    paragraph: "p", heading: "h3", quote: "blockquote", code_block: "pre",
    list: "ul", ordered_list: "ol", item: "li", table: "table", row: "tr", cell: "td",
    strong: "strong", emphasis: "em", strike: "s", code: "code",
  };
  function node(value) {
    if (typeof value === "string") return document.createTextNode(value);
    const target = value.tag === "link" ? value.target : null;
    let element;
    if (target?.kind === "web") {
      try {
        const url = new URL(target.url);
        if (["https:", "http:"].includes(url.protocol) && !url.username && !url.password) {
          element = el("a");
          element.href = url.href;
          element.target = "_blank";
          element.rel = "noopener noreferrer";
        }
      } catch {}
    } else if (target?.kind === "project") {
      element = button("", () => openProject(target.id, element.textContent), "question-project-link");
      element.dataset.scope = "diff";
      if (!scopes().includes("diff")) element.title = "This device has no project file access.";
    }
    element ??= el(Object.hasOwn(tags, value.tag) ? tags[value.tag] : "span");
    if (value.tag === "ordered_list" && Number.isSafeInteger(value.start)) element.start = value.start;
    element.append(...value.children.map(node));
    if (value.tag === "table") {
      const scroll = el("div", undefined, "question-table");
      scroll.append(element);
      return scroll;
    }
    return element;
  }
  return markdown.map(node);
}

function questionCard(threadId, request) {
  const key = JSON.stringify([threadId, request.turn_generation, request.request_id, request.review_hash]);
  const draft = questionDrafts.get(key) || {
    expanded: true,
    answers: request.questions.map(() => ({ selected: [], text: "" })),
    files: request.questions.map(() => []),
    fileErrors: request.questions.map(() => ""),
    transferring: false,
  };
  questionDrafts.delete(key);
  questionDrafts.set(key, draft);
  if (questionDrafts.size > 50) questionDrafts.delete(questionDrafts.keys().next().value);

  const card = el("section", undefined, "approval"), body = el("div");
  const toggle = button("Question from the agent", () => {
    draft.expanded = !draft.expanded;
    toggle.setAttribute("aria-expanded", String(draft.expanded));
    body.hidden = !draft.expanded;
  });
  toggle.setAttribute("aria-expanded", String(draft.expanded));
  body.hidden = !draft.expanded;
  card.append(toggle, body);

  const current = () => view.kind === "thread" && view.id === threadId
    && snapshot?.question?.request_id === request.request_id
    && snapshot.question.turn_generation === request.turn_generation
    && snapshot.question.review_hash === request.review_hash;
  const submit = reply => {
    if (!current()) return;
    sendCommand(command({
      kind: "answer_question", thread_id: threadId, turn_generation: request.turn_generation,
      request_id: request.request_id, review_hash: request.review_hash, reply,
    }));
  };
  const openProject = (link, title) => {
    if (!current() || !scopes().includes("diff")) return;
    openText({
      kind: "question-link", id: threadId, offset: 0, title,
      link, request_id: request.request_id, review_hash: request.review_hash,
      turn_generation: request.turn_generation,
    });
  };
  const appendMarkdown = (group, text, markdown, className) => {
    if (!markdown?.length) {
      group.append(el("p", text, className));
      return;
    }
    const content = el("div", undefined, `${className} question-markdown`);
    content.append(...questionMarkup(markdown, openProject));
    group.append(content);
  };
  const send = button(request.responding ? "Sending answers…" : "Send answers", () => {
    const answers = draft.answers.map((answer, index) => {
      const ids = draft.files[index].filter(entry => entry.state === "ready").map(entry => entry.id);
      return ids.length ? { ...answer, attachments: ids } : answer;
    });
    submit({ type: "answer", answers });
  }, "primary");
  const skip = button("Skip", () => submit({ type: "skip" }));
  const sizeError = el("p", "Shorten each answer to 8 KiB.", "error");
  for (const control of [send, skip]) control.dataset.mutation = "prompt";
  const fileControls = [];
  const paint = () => {
    sizeError.hidden = draft.answers.every(answer => new TextEncoder().encode(answer.text).length <= 8192);
    const complete = draft.answers.every((answer, index) => (
      draft.files[index].every(entry => entry.state === "ready")
      && (answer.selected.length || answer.text.trim() || draft.files[index].length)
    ));
    send.dataset.blocked = String(request.responding || draft.transferring || !sizeError.hidden || !complete);
    skip.dataset.blocked = String(request.responding || draft.transferring);
    fileControls.forEach(render => render());
    controls();
  };
  draft.paint = paint;

  async function transfer(position, entries, upload) {
    if (draft.transferring || !current() || !online || !scopes().includes("prompt")) return;
    const device = session?.device.id;
    draft.transferring = true;
    draft.fileErrors[position] = "";
    try {
      for (const entry of entries) {
        if (!device || session?.device.id !== device) break;
        entry.state = upload ? "uploading" : "removing";
        draft.paint();
        try {
          await api(questionFilePath(threadId, request, position, entry, upload), upload ? {
            method: "POST", body: entry.file, headers: {"Content-Type":"application/octet-stream"}, timeout: 200000,
          } : { method: "DELETE" });
          if (upload) entry.state = "ready";
          else draft.files[position] = draft.files[position].filter(item => item !== entry);
        } catch (error) {
          entry.state = upload ? "failed" : entry.beforeRemoval;
          draft.fileErrors[position] = error.message;
        }
      }
    } finally {
      draft.transferring = false;
      draft.paint();
    }
  }
  request.questions.forEach((question, position) => {
    const answer = draft.answers[position], group = el("section", undefined, "mobile-question");
    const markup = request.markdown?.[position];
    if (question.header) group.append(el("h2", question.header));
    appendMarkdown(group, question.question, markup?.question, "question-body");
    if (question.multiSelect) group.append(el("p", "Choose one or more", "muted"));
    const options = [];
    question.options.forEach((option, index) => {
      const choice = button("", () => {
        answer.selected = question.multiSelect
          ? answer.selected.includes(index) ? answer.selected.filter(value => value !== index) : [...answer.selected, index]
          : [index];
        options.forEach((control, i) => control.setAttribute("aria-pressed", String(answer.selected.includes(i))));
        paint();
      }, "question-choice");
      choice.append(el("strong", option.label));
      if (option.description) choice.append(el("small", option.description));
      choice.setAttribute("aria-pressed", String(answer.selected.includes(index)));
      choice.dataset.mutation = "prompt";
      choice.dataset.blocked = String(request.responding);
      options.push(choice);
      group.append(choice);
      if (option.preview) appendMarkdown(group, option.preview, markup?.previews[index], "muted question-preview");
    });
    if (question.allowCustom) {
      const label = el("label", "Custom answer"), input = el("textarea");
      input.id = `custom-answer-${position}`;
      input.maxLength = 8192;
      input.value = answer.text;
      input.dataset.mutation = "prompt";
      input.dataset.blocked = String(request.responding);
      input.addEventListener("input", () => { answer.text = input.value; paint(); });
      label.htmlFor = input.id;
      group.append(label, input);
      if (request.can_attach) {
        const fileInput = el("input");
        fileInput.type = "file";
        fileInput.multiple = true;
        fileInput.hidden = true;
        fileInput.setAttribute("aria-label", "Attach files");
        const attach = button("Attach files", () => fileInput.click());
        const files = el("div", undefined, "question-files");
        const error = el("p", undefined, "error");
        error.setAttribute("role", "status");
        attach.dataset.mutation = "prompt";
        fileInput.addEventListener("change", () => {
          const picked = Array.from(fileInput.files);
          fileInput.value = "";
          if (!picked.length || draft.transferring || !current()) return;
          if (draft.files.flat().length + picked.length > 100) {
            draft.fileErrors[position] = "Add at most 100 files to the answer.";
            paint();
            return;
          }
          const entries = picked.map(file => ({ id: crypto.randomUUID(), file, state: "queued" }));
          draft.files[position].push(...entries);
          void transfer(position, entries, true);
        });
        fileControls.push(() => {
          const blocked = String(request.responding || draft.transferring);
          attach.dataset.blocked = blocked;
          error.hidden = !draft.fileErrors[position];
          error.textContent = draft.fileErrors[position];
          files.replaceChildren(...draft.files[position].map(entry => {
            const row = el("div", undefined, "question-file");
            row.append(el("strong", entry.file.name));
            const actions = el("div", undefined, "row");
            if (entry.state === "failed" || entry.state === "queued") {
              actions.append(button("Retry", () => transfer(position, [entry], true)));
            } else {
              const status = { uploading: "Uploading…", removing: "Removing…", ready: "Ready" };
              const label = el("small", status[entry.state], "muted");
              label.setAttribute("role", "status");
              actions.append(label);
            }
            const remove = button("Remove", () => {
              entry.beforeRemoval = entry.state;
              void transfer(position, [entry], false);
            });
            remove.setAttribute("aria-label", `Remove ${entry.file.name}`);
            actions.append(remove);
            actions.querySelectorAll("button").forEach(control => {
              control.dataset.mutation = "prompt";
              control.dataset.blocked = blocked;
            });
            row.append(actions);
            return row;
          }));
        });
        group.append(fileInput, attach, files, error);
      }
    }
    body.append(group);
  });
  if (request.error) body.append(el("p", request.error, "error"));
  if (!scopes().includes("prompt")) body.append(el("p", "Send prompts was not granted for this device.", "muted"));
  const row = el("div", undefined, "row");
  row.append(send, skip);
  body.append(sizeError, row);
  paint();
  return card;
}
function requestRefresh(){
  if(stopped||!token)return;
  if(refreshTask){refreshAgain=true;return;}
  const g=generation, current={...view};
  refreshTask=(async()=>{
    try{
      if(current.kind==="threads"){const result=await api(`/api/mobile/threads?offset=${current.offset||0}`);if(g===generation)renderThreads(result.value);}
      else if(current.kind==="thread"){const q=new URLSearchParams({thread_id:current.id});if(current.before!==undefined)q.set("before",current.before);const result=await api(`/api/mobile/thread?${q}`);if(g===generation)renderThread(result.value);}
      else await api("/api/mobile/session");
      if(!stopped){online=true;controls();}
    }catch(error){if(!stopped){online=false;controls();notice(error.message);}}
    finally{refreshTask=null;if(refreshAgain){refreshAgain=false;requestRefresh();}}
  })();
}
async function syncSession(){
  session=await api("/api/mobile/session");
  if(!restoredCommand){
    restoredCommand=true;
    try{const saved=JSON.parse(sessionStorage.getItem(commandKey)||"null");if(saved?.device===session.device.id&&saved.command?.id&&Number.isSafeInteger(saved.command.sequence)&&saved.command.action){pendingCommand=saved.command;}else if(saved){sessionStorage.removeItem(commandKey);}}
    catch{status="A prior command could not be recovered. Check the laptop before sending it again.";}
    if(!pendingCommand&&session.last_command)recordReceipt(session.last_command);
  }
  if(pendingCommand){
    const last=session.last_command;
    if(last?.id===pendingCommand.id&&last.sequence===pendingCommand.sequence)recordReceipt(last);
    else if(session.next_sequence!==pendingCommand.sequence){status="This command's delivery cannot be established. Check the laptop before sending it again.";pendingCommand=null;try{sessionStorage.removeItem(commandKey);}catch{}}
  }
  renderDelivery();
}
function recordReceipt(receipt){
  const outcome=receipt.outcome;
  if(outcome.status==="pending"){status="Reserved on the desktop; delivery is pending. Checking the same command cannot run it twice.";}
  else if(outcome.status==="accepted"){
    status=outcome.queued?"Accepted into the desktop queue.":"Accepted by the desktop.";
    if(pendingCommand?.action.kind==="prompt"){
      const action=pendingCommand.action;
      if((drafts.get(action.thread_id)||"").trim()===action.text)drafts.delete(action.thread_id);
      if(view.kind==="thread"&&view.id===action.thread_id&&promptDraft.trim()===action.text){promptDraft="";const input=document.querySelector("#prompt");if(input)input.value="";}
    }
    pendingCommand=null;
  }else{status=outcome.status==="rejected"?outcome.reason:"Delivery is unconfirmed. This command will not be replayed. Check the laptop.";pendingCommand=null;}
  if(session)session.next_sequence=Math.max(session.next_sequence,receipt.sequence+1);
  if(!pendingCommand){try{sessionStorage.removeItem(commandKey);}catch{}}
  renderDelivery();
}
async function sendCommand(command){
  if(!online||sending)return;
  try{sessionStorage.setItem(commandKey,JSON.stringify({device:session.device.id,command}));}
  catch{status="This browser cannot retain a delivery receipt across reloads. Enable tab storage before sending a command.";renderDelivery();return;}
  sending=true;pendingCommand=command;status="Sending…";renderDelivery();
  try{const receipt=await api("/api/mobile/command",{method:"POST",body:JSON.stringify(command)});recordReceipt(receipt);await syncSession();requestRefresh();}
  catch(error){if(!stopped){
    status=`${error.message} Delivery may be pending; check the same command before sending another.`;online=false;controls();
    try{
      await syncSession();
      if([400,403].includes(error.status)&&pendingCommand?.id===command.id&&session.next_sequence===command.sequence&&session.last_command?.id!==command.id){pendingCommand=null;try{sessionStorage.removeItem(commandKey);}catch{}status=error.message;}
    }catch{}
  }}
  finally{sending=false;renderDelivery();}
}
function command(action){return{id:crypto.randomUUID(),sequence:session.next_sequence,action};}
function sendPrompt(){const text=promptDraft.trim();if(!text)return;if(new TextEncoder().encode(text).length>32768){status="Shorten the prompt to 32 KiB.";renderDelivery();return;}sendCommand(command({kind:"prompt",thread_id:view.id,text}));}
function permission(p,decision){sendCommand(command({kind:"permission",thread_id:view.id,turn_generation:p.turn_generation,request_id:p.request_id,review_hash:p.review_hash,decision}));}
function openDiff(){if(view.kind!=="thread")return;openText({kind:"diff",id:view.id,offset:0});}
function openMessage(m){openText({kind:"message",id:view.id,index:m.index,reference:m.reference,offset:0});}
async function openText(next){
  rememberDraft();
  const title=next.kind==="question-link"?next.title:next.kind==="diff"?"Working tree diff":"Full message";
  view=next;generation++;const g=generation;app.replaceChildren(button("← Thread",()=>setView({kind:"thread",id:next.id})),el("h1",title));controls();
  try{
    const q=new URLSearchParams({thread_id:next.id,offset:next.offset});if(next.revision)q.set("revision",next.revision);if(next.kind==="message"){q.set("index",next.index);q.set("reference",next.reference);}
    if(next.kind==="question-link")for(const field of ["link","request_id","review_hash","turn_generation"])q.set(field,next[field]);
    const result=await api(`/api/mobile/${next.kind}?${q}`);if(g!==generation)return;const page=result.value;
    app.append(el("pre",page.text||(next.kind==="question-link"?"This file is empty.":"No changes."),"text-page"));
    if(page.source_truncated)app.append(el("p","This preview reached the desktop size limit. Review the rest on the laptop.","notice"));
    const row=el("div",undefined,"row");if(next.offset)row.append(button("First page",()=>openText({...next,offset:0,revision:undefined})));if(page.next_offset!==null)row.append(button("Next page",()=>openText({...next,offset:page.next_offset,revision:page.revision})));row.append(button("Reload",()=>openText({...next,offset:0,revision:undefined})));app.append(row);online=true;controls();
  }catch(error){if(g===generation&&!stopped){app.append(el("p",error.message,"error"));app.append(button("Return to latest thread",()=>setView({kind:"thread",id:next.id})));}}
}
async function streamLoop(){
  let delay=1000;
  while(!stopped&&token){
    try{
      await syncSession();
      streamController=new AbortController();
      const response=await fetch("/api/mobile/events",{headers:{Authorization:`Bearer ${token}`},credentials:"omit",cache:"no-store",redirect:"error",signal:streamController.signal});
      if(response.status===401){forget("This device is no longer authorized. Pair again from the laptop.");return;}
      if(!response.ok||!response.body)throw new Error("The desktop stream is unavailable.");
      const reader=response.body.getReader(),decoder=new TextDecoder();let buffer="";delay=1000;requestRefresh();
      for(;;){const {value,done}=await reader.read();if(done)break;buffer+=decoder.decode(value,{stream:true});if(buffer.length>4096)throw new Error("Invalid desktop stream.");let split;
        while((split=buffer.indexOf("\n\n"))>=0){const event=buffer.slice(0,split);buffer=buffer.slice(split+2);if(event.includes("event: revoked")){forget("This device was revoked, expired, or mobile access was stopped. Pair again from the laptop.");return;}if(event.includes("event: refresh"))requestRefresh();}
      }
    }catch(error){if(stopped)return;}
    finally{streamController?.abort();streamController=null;}
    online=false;controls();if(stopped)return;
    await new Promise(resolve=>setTimeout(resolve,delay));delay=Math.min(delay*2,30000);
  }
}
async function pairing(secret){
  app.replaceChildren(el("h1","Pair with your laptop"),el("p","Only continue if you just opened this QR code in Bruhs on your own laptop. The link can be used once.","muted"));
  const label=el("label","Device name");label.htmlFor="device-name";const input=el("input");input.id="device-name";input.maxLength=128;input.autocomplete="off";input.placeholder="My phone";
  const submit=button("Pair this device",async()=>{submit.disabled=true;try{const result=await api("/api/mobile/pair",{method:"POST",body:JSON.stringify({secret,name:input.value.trim()})});secret="";token=result.token;try{localStorage.setItem(storageKey,token);}catch{notice("Browser storage is unavailable; this pairing lasts until this page closes.");}session={device:result.device,next_sequence:result.next_sequence,last_command:null};stopped=false;setView({kind:"threads",offset:0});streamLoop();}catch(error){notice(error.message);submit.disabled=false;}},"primary");
  app.append(label,input,el("div",undefined,"row"));app.lastChild.append(submit);controls();
}
document.querySelector("#home").addEventListener("click",event=>{event.preventDefault();if(token&&!stopped)setView({kind:"threads",offset:0});});
disconnect.addEventListener("click",async()=>{disconnect.disabled=true;try{await api("/api/mobile/disconnect",{method:"POST"});forget("This phone was revoked and its local token cleared.");}catch(error){if(!stopped)notice(`${error.message} Revoke this device from the laptop to end access.`);}finally{disconnect.disabled=false;}});
window.addEventListener("offline",()=>{online=false;streamController?.abort();controls();});
window.addEventListener("pagehide",()=>{online=false;streamController?.abort();controls();});
let pairSecret=new URLSearchParams(location.hash.slice(1)).get("pair");
history.replaceState(null,"",location.pathname);
if(pairSecret){pairing(pairSecret);pairSecret=null;}else{
  try{token=localStorage.getItem(storageKey)||"";}catch{}
  if(token){setView({kind:"threads",offset:0});streamLoop();}else{stopped=true;app.append(el("h1","Open Bruhs on your laptop"),el("p","Start mobile access, choose this device’s permissions, then scan the pairing QR code.","muted"));controls();}
}
