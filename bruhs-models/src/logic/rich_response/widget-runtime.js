(() => {
  let state = { fields: {}, custom: null, scroll_x: 0, scroll_y: 0 }, initialized = false, scheduled = false;
  const restored = new WeakSet();
  const sensitive = new WeakSet();
  let changes, sizes;
  const credentialAutocomplete = value => String(value || '').toLowerCase().split(/\s+/).some(token => ['current-password','new-password','one-time-code'].includes(token) || token.startsWith('cc-'));
  const processRecords = records => {
    for(const record of records){
      if(record.type==='attributes'&&((record.attributeName==='type'&&['password','file'].includes(String(record.oldValue).toLowerCase()))||(record.attributeName==='autocomplete'&&credentialAutocomplete(record.oldValue))))sensitive.add(record.target);
      if(sizes)for(const element of record.addedNodes){if(element.nodeType===1){sizes.observe(element);for(const child of element.querySelectorAll('*'))sizes.observe(child);}}
    }
  };
  const emit = data => parent.postMessage({ ...data, bruhs: 1 }, '*');
  let resolveReady;
  const ready = new Promise(resolve => { resolveReady = resolve; });
  const controls = () => { if(changes)processRecords(changes.takeRecords());return [...document.querySelectorAll('input,textarea,select,[contenteditable="true"]')]
    .filter(element => {
      if(['password','file'].includes(element.type) || credentialAutocomplete(element.autocomplete))sensitive.add(element);
      return !sensitive.has(element) && !element.hasAttribute('data-bruhs-no-persist');
    }); };
  const key = (element, index) => element.getAttribute('data-bruhs-key') || (element.id ? `id:${element.id}` : `field:${element.name || element.tagName}:${index}`);
  const snapshot = () => {
    const fields = Object.create(null);
    controls().forEach((element, index) => { fields[key(element,index)] = {
      value: element.isContentEditable ? element.innerText : element.value,
      checked: !!element.checked,
      selected: element.selectedOptions ? [...element.selectedOptions].map(option => option.value) : []
    }; });
    return { fields, custom: state.custom, scroll_x: scrollX, scroll_y: scrollY };
  };
  const reportState = (immediate=false) => {
    if (!initialized || (!immediate && scheduled)) return;
    const send=()=>{scheduled=false;try{state=snapshot();const json=JSON.stringify(state);if(new TextEncoder().encode(json).length>262144)throw Error('Widget state exceeds the 256 KiB response-state budget.');emit({type:'state',state});}catch(error){emit({type:'error',message:String(error.message||error)});}};
    if(immediate)send();else{scheduled=true;requestAnimationFrame(send);}
  };
  const restore = () => controls().forEach((element,index) => {
    if(restored.has(element)) return;
    restored.add(element);
    const field=state.fields?.[key(element,index)];
    if(!field) return;
    if(element.isContentEditable) element.innerText=String(field.value ?? ''); else element.value=String(field.value ?? '');
    if('checked' in element) element.checked=!!field.checked;
    if(element.options && Array.isArray(field.selected)) [...element.options].forEach(option=>{option.selected=field.selected.includes(option.value)});
  });
  const reportSize = () => {
    if(!document.body) return;
    const body=document.body.getBoundingClientRect();let height=body.height;
    for(const element of document.body.querySelectorAll('*')){
      if(['SCRIPT','STYLE','LINK','TEMPLATE'].includes(element.tagName))continue;
      const style=getComputedStyle(element);if(style.display==='none'||style.visibility==='hidden')continue;
      const rect=element.getBoundingClientRect();height=Math.max(height,rect.bottom+scrollY-body.top);
    }
    height=Math.ceil(height);
    if(Number.isFinite(height)) emit({type:'resize',height:Math.max(80,Math.min(20000,height))});
  };
  window.bruhs = Object.freeze({
    ready,
    getState: () => state.custom,
    setState: value => { try { state.custom=JSON.parse(JSON.stringify(value));reportState(true); } catch(error){emit({type:'error',message:String(error.message || error)})} },
    submit: value => {
      const action=typeof value==='string'?{label:'Use in chat',message:value}:value;
      if(!action || typeof action.message!=='string' || !action.message.trim() || action.message.length>262144)throw Error('A widget action needs a chat draft within the response-state budget.');
      emit({type:'action',action:{label:typeof action.label==='string'?action.label:'Use in chat',message:action.message}});
    }
  });
  addEventListener('message',event=>{
    if(event.source!==parent || event.data?.bruhs!==1)return;
    if(event.data.type==='flush'){reportState(true);return;}
    if(event.data.type!=='init')return;
    const data=event.data;
    if(!initialized){state=data.state || state;initialized=true;restore();scrollTo(state.scroll_x || 0,state.scroll_y || 0);resolveReady(state.custom);dispatchEvent(new CustomEvent('bruhs:restore',{detail:state.custom}));}
    for(const [name,value] of Object.entries(data.theme || {}))if(typeof value==='string'&&(name.startsWith('--')||name==='color-scheme'))document.documentElement.style.setProperty(name,value);
    reportSize();
  });
  addEventListener('error',event=>emit({type:'error',message:String(event.message || 'Widget script failed.')}));
  addEventListener('unhandledrejection',event=>emit({type:'error',message:String(event.reason?.message || event.reason || 'Widget promise rejected.')}));
  addEventListener('input',()=>reportState(true));addEventListener('change',()=>reportState(true));addEventListener('scroll',()=>reportState(false));
  changes=new MutationObserver(records=>{processRecords(records);if(initialized)restore();controls();reportSize();});
  changes.observe(document.documentElement,{childList:true,subtree:true,attributes:true,attributeOldValue:true,attributeFilter:['type','autocomplete','style','class','width','height','hidden','open']});
  addEventListener('DOMContentLoaded',()=>{
    controls();
    sizes=new ResizeObserver(reportSize);sizes.observe(document.body);
    for(const element of document.body.querySelectorAll('*'))sizes.observe(element);
    emit({type:'ready'});reportSize();
  });
})();
