// Benchmark-only SDK host. No production Bruhs entry imports this package.
import { createRoot } from 'react-dom/client';
import { getAssetUrls } from '@tldraw/assets/selfHosted';
import { Tldraw, createShapeId, createBindingId, toRichText, getArrowInfo, serializeTldrawJson, Editor, type TLShapePartial, type TLBindingCreate, type TLArrowBinding } from 'tldraw';
import 'tldraw/tldraw.css';
type Point = { x: number; y: number };
type Endpoint = { point: Point; binding?: { shape_id: string; anchor: Point } | null };
type Shape = {
  id: string; z: number; rect: { x: number; y: number; w: number; h: number }; rotation: number; text: string;
  kind: { type: 'rectangle' | 'text' } | { type: 'arrow'; start: Endpoint; end: Endpoint };
};
type Scene = { id: string; shapes: Record<string, Shape> };
type Camera = { x: number; y: number; zoom: number };
let editor: Editor | null = null, scene: Scene | null = null, updates = 0;
let baseline = new Map<string, { x: number; y: number }>();
const fonts = Object.fromEntries(['mono', 'serif', 'sans', 'draw'].flatMap(family => ['', '_italic', '_bold', '_italic_bold'].map(style => [`tldraw_${family}${style}`, '/fonts/MonaspaceArgonVariable.woff2'])));
const assets = { ...getAssetUrls({ baseUrl: '/tldraw-assets/' }), fonts };
function current(): Editor { if (!editor) throw new Error('Comparator not mounted'); return editor; }
function point(endpoint: Endpoint): Point {
  if (!endpoint.binding) return endpoint.point;
  const target = scene?.shapes[endpoint.binding.shape_id];
  if (!target) throw new Error('Missing fixture binding');
  return { x: target.rect.x + target.rect.w * endpoint.binding.anchor.x, y: target.rect.y + target.rect.h * endpoint.binding.anchor.y };
}
function shapeRecord(shape: Shape): TLShapePartial {
  const base = { id: createShapeId(shape.id), x: shape.rect.x, y: shape.rect.y, rotation: shape.rotation };
  switch (shape.kind.type) {
    case 'rectangle': return { ...base, type: 'geo', props: { geo: 'rectangle', w: shape.rect.w, h: shape.rect.h, dash: 'solid', color: 'black', fill: 'semi', size: 's', font: 'mono', richText: toRichText(shape.text), align: 'middle', verticalAlign: 'middle' } };
    case 'text': return { ...base, type: 'text', props: { w: shape.rect.w, autoSize: false, size: 's', font: 'mono', color: 'black', richText: toRichText(shape.text) } };
    case 'arrow': return { ...base, type: 'arrow', x: 0, y: 0, props: { kind: 'arc', start: point(shape.kind.start), end: point(shape.kind.end), bend: 0, dash: 'solid', size: 's', color: 'black', fill: 'none', arrowheadStart: 'none', arrowheadEnd: 'triangle' } };
  }
}
function bindingRecords(shape: Shape): TLBindingCreate<TLArrowBinding>[] {
  if (shape.kind.type !== 'arrow') return [];
  const records: TLBindingCreate<TLArrowBinding>[] = [];
  for (const terminal of ['start', 'end'] as const) {
    const endpoint = shape.kind[terminal];
    if (!endpoint.binding) throw new Error('Fixture endpoint must bind');
    records.push({ id: createBindingId(`${shape.id}-${terminal}`), type: 'arrow', fromId: createShapeId(shape.id), toId: createShapeId(endpoint.binding.shape_id), props: { terminal, normalizedAnchor: endpoint.binding.anchor, isExact: true, isPrecise: true, snap: 'none' } });
  }
  return records;
}
async function load(value: Scene) {
  const e = current(); scene = value; updates = 0;
  const shapes = Object.values(value.shapes).sort((a, b) => a.z - b.z || a.id.localeCompare(b.id));
  e.run(() => { e.deleteShapes([...e.getCurrentPageShapeIds()]); e.createShapes(shapes.map(shapeRecord)); e.createBindings(shapes.flatMap(bindingRecords)); });
  await e.fonts.loadRequiredFontsForCurrentPage(); await document.fonts.ready;
  e.selectNone(); e.setCurrentTool('select'); e.updateInstanceState({ isGridMode: false });
  if (e.getCurrentPageShapes().length !== shapes.length) throw new Error('Comparator omitted shapes');
  const bindings = e.store.allRecords().filter(record => record.typeName === 'binding');
  if (bindings.length !== shapes.filter(s => s.kind.type === 'arrow').length * 2) throw new Error('Comparator omitted bindings');
  return stats();
}
function camera(value: Camera): void { current().setCamera({ x: -value.x, y: -value.y, z: value.zoom }, { immediate: true, force: true }); updates++; }
function begin(ids: string[]): void {
  const e = current(); e.select(...ids.map(id => createShapeId(id)));
  baseline = new Map(ids.map(id => { const shape = e.getShape(createShapeId(id)); if (!shape) throw new Error('Missing selected shape'); return [id, { x: shape.x, y: shape.y }]; }));
}
function movement(delta: Point): void {
  const e = current();
  e.run(() => e.updateShapes([...baseline].map(([id, p]): TLShapePartial => {
    const shape = e.getShape(createShapeId(id)); if (!shape) throw new Error('Selected shape disappeared');
    return { id: shape.id, type: shape.type, x: p.x + delta.x, y: p.y + delta.y };
  }))); updates++;
}
function end(): void { baseline.clear(); }
function stats() {
  const e = current(), c = e.getCamera(), viewport = e.getViewportPageBounds(), shapes = e.getCurrentPageShapes();
  const arrows = shapes.flatMap(shape => {
    if (shape.type !== 'arrow') return [];
    const info = getArrowInfo(e, shape), transform = e.getShapePageTransform(shape);
    if (!info || !transform) throw new Error('A rendered arrow has no terminal geometry');
    return [{ id: shape.id.slice(6), start: transform.applyToPoint(info.start.handle), end: transform.applyToPoint(info.end.handle) }];
  });
  return { renderer: 'tldraw-5.6.0', shapes: shapes.length, bindings: e.store.allRecords().filter(r => r.typeName === 'binding').length, updates, camera: { x: -c.x, y: -c.y, zoom: c.z }, viewport: { x: viewport.x, y: viewport.y, w: viewport.w, h: viewport.h }, geometry: shapes.filter(s => s.type !== 'arrow').map(s => ({ id: s.id.slice(6), x: s.x, y: s.y })), arrows, visible: shapes.filter(s => { const b = e.getShapePageBounds(s); return b && b.collides(viewport); }).length };
}
Object.assign(window, { __boardBench: { ready: () => !!editor, exportFile: () => serializeTldrawJson(current()), load, camera, begin, movement, end, stats, frame: () => new Promise(resolve => requestAnimationFrame(resolve)) } });
const root = document.getElementById('root'); if (!root) throw new Error('Comparator root is missing');
createRoot(root).render(<Tldraw hideUi autoFocus options={{ maxShapesPerPage: 20000 }} assetUrls={assets} onMount={value => { editor = value; value.user.updateUserPreferences({ colorScheme: 'light', isSnapMode: false }); }} />);
