// Permission dialogs and offline-model lookups may resolve after cancellation,
// backgrounding, unmount, or a new attempt. Only the current foreground attempt
// may turn on the microphone.
export class RecordingGate {
  private generation = 0;
  private foreground = false;
  private live = false;
  private preparing = false;

  mount(foreground: boolean): void {
    this.generation++;
    this.foreground = foreground;
    this.live = true;
    this.preparing = false;
  }
  unmount(): void {
    this.cancel();
    this.live = false;
  }
  setForeground(foreground: boolean): void {
    this.foreground = foreground;
    if (!foreground) this.cancel();
  }
  cancel(): void {
    this.generation++;
    this.preparing = false;
  }
  begin(): number | null {
    if (!this.live || !this.foreground || this.preparing) return null;
    this.preparing = true;
    return ++this.generation;
  }
  current(generation: number): boolean {
    return this.live && this.foreground && generation === this.generation;
  }
  finish(generation: number): boolean {
    if (!this.current(generation)) return false;
    this.preparing = false;
    return true;
  }
  get mounted(): boolean {
    return this.live;
  }
  get starting(): boolean {
    return this.preparing;
  }
}
