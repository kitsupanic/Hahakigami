/** Keep at most one running query and one replacement for rapidly changing views. */
export function latestOnly() {
  let pending: (() => Promise<void>) | null = null;
  let running = false;
  return (job: () => Promise<void>) => {
    pending = job;
    if (running) return;
    running = true;
    void (async () => {
      try {
        while (pending) {
          const next = pending;
          pending = null;
          try {
            await next();
          } catch (error) {
            console.error(error);
          }
        }
      } finally {
        running = false;
      }
    })();
  };
}
