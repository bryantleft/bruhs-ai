(() => {
  let frame, nonce = '', initialization, ready = false;
  const emit = data => {
    let safe;
    switch(data.type){
      case 'ready':safe={type:'ready'};break;
      case 'resize':if(!Number.isFinite(data.height))return;safe={type:'resize',height:data.height};break;
      case 'state':safe={type:'state',state:data.state};break;
      case 'action':if(typeof data.action?.message!=='string'||!data.action.message.trim())return;safe={type:'action',action:{label:typeof data.action.label==='string'?data.action.label:'Use in chat',message:data.action.message}};break;
      case 'error':if(typeof data.message!=='string')return;safe={type:'error',message:data.message};break;
      default:return;
    }
    try{if(new TextEncoder().encode(JSON.stringify(safe)).length>262144)throw Error('Widget message is too large.');}
    catch{safe={type:'error',message:'The widget sent an invalid or oversized state or action.'};}
    const packet = { ...safe, bruhs: 1, nonce };
    if (parent !== window) parent.postMessage(packet, '*');
    if (window.ipc && typeof window.ipc.postMessage === 'function') window.ipc.postMessage(JSON.stringify(packet));
  };
  window.bruhsHostInit = data => {
    if (!data || data.bruhs !== 1 || data.type !== 'init') return;
    initialization = data;
    if (ready) frame.contentWindow.postMessage(initialization, '*');
  };
  window.bruhsHostFlush = () => { if(ready && frame)frame.contentWindow.postMessage({bruhs:1,type:'flush'},'*'); };
  addEventListener('message', event => {
    if (event.source === parent && event.data?.bruhs === 1 && event.data?.type === 'init') {
      window.bruhsHostInit(event.data);
      return;
    }
    if (!frame || event.source !== frame.contentWindow || event.data?.bruhs !== 1) return;
    const data = event.data;
    if (!['ready', 'resize', 'state', 'error', 'action'].includes(data.type)) return;
    if (data.type === 'ready') {
      if(ready)return;
      ready = true;
      if (initialization) frame.contentWindow.postMessage(initialization, '*');
    }
    emit(data);
  });
  window.bruhsCreateWidget = payload => {
    if (!payload || typeof payload.html !== 'string' || typeof payload.nonce !== 'string' || payload.nonce.length>64) throw Error('Invalid widget');
    nonce = payload.nonce;
    frame = document.createElement('iframe');
    frame.setAttribute('sandbox', 'allow-scripts');
    frame.setAttribute('allow', "camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'; display-capture 'none'");
    frame.setAttribute('referrerpolicy', 'no-referrer');
    frame.title = 'Interactive response';
    frame.style.cssText = 'border:0;width:100%;height:100%';
    frame.srcdoc = payload.html;
    (document.getElementById('bruhs-rich-root') || document.body).append(frame);
  };
})();
