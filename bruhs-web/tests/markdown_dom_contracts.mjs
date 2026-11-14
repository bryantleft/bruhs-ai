// Browser-only contract fixture. Run in a disposable browser context with
// the packaged patchSanitizedMarkdown export and that context's document.
// Throws on failure and returns the names of all passing checks.
export function verifyMarkdownDom(patchSanitizedMarkdown, document) {
  const checks=[];
  const check=(ok,name)=>{if(!ok)throw new Error(name);checks.push(name);};
  const mount=document.createElement('div');mount.style.cssText='position:fixed;top:-10000px';
  document.body.append(mount);
  try {
  const cases=[
   ['<p>plain</p>\n','<h2>heading</h2>\n<p>tail</p>\n'],
   ['<p>leading</p>\n<p>middle</p>\n<p>trailing</p>\n','<p>leading</p>\n<p>trailing</p>\n'],
   ['<blockquote><p>quote <em>old</em></p></blockquote>','<blockquote><p>quote <strong>new</strong></p><ul><li>one</li><li>two</li></ul></blockquote>'],
   ['<p>see [guide] and [two]</p>','<p>see <a href="/guide" title="new">guide</a> and <a href="/two">two</a></p>'],
   ['<p><a href="/old" title="obsolete" class="old">same</a></p>','<p><a href="/new">same</a></p>'],
   ['<p>literal &#96;code</p>','<p>literal <code>code</code></p>'],
   ['<pre><code class="language-rust">let x = 1;\n</code></pre>','<pre><code class="language-rust">let x = 1;\nlet y = 2;\n</code></pre>'],
   ['<p>table | head</p>','<table><thead><tr><th align="left">head</th><th>two</th></tr></thead><tbody><tr><td align="left">a</td><td>b</td></tr></tbody></table>'],
   ['<ul><li><input disabled="" type="checkbox"/> task</li></ul>','<ul><li><input disabled="" type="checkbox" checked=""/> task</li><li>next</li></ul>'],
   ['<p>one<sup class="footnote-reference"><a href="#a">1</a></sup></p><div class="footnote-definition" id="a"><sup class="footnote-definition-label">1</sup><p>old</p></div>','<p>one<sup class="footnote-reference"><a href="#b">2</a></sup></p><div class="footnote-definition" id="b"><sup class="footnote-definition-label">2</sup><p>new</p></div>'],
   ['<p>emoji 🗒 &amp; escaped &lt;script&gt;</p>','<p>emoji 🔭 &amp; escaped &lt;script&gt; appended</p>'],
   ['<p>old</p>',''],['','<p>new</p>'],['<hr/>','<p>after rule</p>']
  ];
  cases.forEach(([before,after],i)=>{mount.innerHTML=before;mount.setAttribute('data-render-pass','');patchSanitizedMarkdown(mount,after);const expected=document.createElement('div');expected.innerHTML=after;check(mount.innerHTML===expected.innerHTML,'complete DOM transition '+i);check(!mount.hasAttribute('data-render-pass'),'render marker cleared '+i)});
  mount.innerHTML='<p>unchanged <strong>anchor</strong> 🔭</p><p>tail</p>';
  const head=mount.firstChild,strong=head.querySelector('strong'),tail=mount.lastChild,tailText=tail.firstChild;
  const selection=document.defaultView.getSelection();const range=document.createRange();range.setStart(head.firstChild,0);range.setEnd(head.firstChild,9);selection.removeAllRanges();selection.addRange(range);
  patchSanitizedMarkdown(mount,'<p>unchanged <strong>anchor</strong> 🔭</p><p>tail plus</p>');
  check(mount.firstChild===head&&head.querySelector('strong')===strong,'unchanged prefix identity');
  check(mount.lastChild===tail&&tail.firstChild===tailText,'appended text identity');
  check(selection.toString()==='unchanged','selection in earlier paragraph survives');
  const tailRange=document.createRange();tailRange.setStart(tailText,0);tailRange.setEnd(tailText,4);selection.removeAllRanges();selection.addRange(tailRange);
  patchSanitizedMarkdown(mount,'<p>unchanged <strong>anchor</strong> 🔭</p><p>tail plus more</p>');
  check(selection.toString()==='tail','selection inside growing text survives');
  selection.removeAllRanges();
  return {passed:checks.length,checks};
  } finally {mount.remove();}
}
