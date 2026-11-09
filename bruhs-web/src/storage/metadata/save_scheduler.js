// Give visible optimistic feedback a rendering opportunity before serializing.
// The deadline is a fallback, not a debounce: hidden pages never wait for rAF.
export function yieldBeforeMetadataSave(scope = globalThis) {
    if (!scope.document || scope.document.hidden || typeof scope.requestAnimationFrame !== 'function') {
        return Promise.resolve();
    }
    return new Promise(resolve => {
        let done = false;
        let frame, deadline, task;
        const finish = () => {
            if (done) return;
            done = true;
            // Resolve even if a browser scheduling/cleanup API fails.
            try {
                scope.document.removeEventListener('visibilitychange', visibility);
                if (frame !== undefined) scope.cancelAnimationFrame(frame);
                if (deadline !== undefined) scope.clearTimeout(deadline);
                if (task !== undefined) scope.clearTimeout(task);
            } catch { /* A failed cleanup must not strand persistence. */ }
            finally { resolve(); }
        };
        const visibility = () => { if (scope.document.hidden) finish(); };
        try {
            scope.document.addEventListener('visibilitychange', visibility);
            deadline = scope.setTimeout(finish, 50);
            frame = scope.requestAnimationFrame(() => {
                if (done) return;
                frame = undefined;
                try { task = scope.setTimeout(finish, 0); }
                catch { finish(); }
            });
        } catch { finish(); }
    });
}
