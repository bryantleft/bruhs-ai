// Input is complete HTML from render_untrusted, never raw Markdown. Keep
// unchanged nodes attached so a new token does not invalidate every paragraph's
// layout or detach the reader's selection. The temporary tree is not mounted.
export function patchSanitizedMarkdown(element, html) {
  const template = element.ownerDocument.createElement('template');
  template.innerHTML = html;
  patchChildren(element, template.content);
  // A changed settled response must receive highlighting/math again. The
  // streaming pass is suspended; an unchanged HTML value retains this marker.
  element.removeAttribute('data-render-pass');
}

function patchChildren(element, desired) {
  let current = element.firstChild;
  let incoming = desired.firstChild;
  while (incoming) {
    const nextIncoming = incoming.nextSibling;
    if (current) {
      const nextCurrent = current.nextSibling;
      patchNode(current, incoming);
      current = nextCurrent;
    } else {
      element.appendChild(incoming);
    }
    incoming = nextIncoming;
  }
  while (current) {
    const next = current.nextSibling;
    element.removeChild(current);
    current = next;
  }
}

function patchNode(current, incoming) {
  if (current.nodeType === Node.ELEMENT_NODE && incoming.nodeType === Node.ELEMENT_NODE
      && current.hasAttribute('data-bruhs-managed')
      && current.getAttribute('data-bruhs-managed') === incoming.getAttribute('data-bruhs-managed')) return;
  if (current.isEqualNode(incoming)) return;
  if (current.nodeType !== incoming.nodeType || current.nodeName !== incoming.nodeName
      || current.namespaceURI !== incoming.namespaceURI) {
    current.replaceWith(incoming);
  } else if (current.nodeType === Node.TEXT_NODE || current.nodeType === Node.COMMENT_NODE) {
    const next = incoming.data;
    if (next.startsWith(current.data)) {
      // appendData keeps ranges in the already visible text at their offsets.
      current.appendData(next.slice(current.length));
    } else {
      current.data = next;
    }
  } else {
    for (const attribute of Array.from(current.attributes)) {
      if (!incoming.hasAttributeNS(attribute.namespaceURI, attribute.localName)) {
        current.removeAttributeNS(attribute.namespaceURI, attribute.localName);
      }
    }
    for (const attribute of incoming.attributes) {
      if (current.getAttributeNS(attribute.namespaceURI, attribute.localName) !== attribute.value) {
        current.setAttributeNS(attribute.namespaceURI, attribute.name, attribute.value);
      }
    }
    patchChildren(current, incoming);
  }
}
