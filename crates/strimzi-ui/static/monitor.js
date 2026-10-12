(() => {
  const session = document.getElementById("monitor-session");
  if (!session) return;

  const heading = document.getElementById("monitor-heading");
  const status = document.getElementById("monitor-status");
  const error = document.getElementById("monitor-error");
  const results = document.getElementById("snapshot-results");
  const stop = document.getElementById("stop-monitor");
  const params = new URLSearchParams({
    topic: session.dataset.topic,
    duration: session.dataset.duration,
  });
  const source = new EventSource(`/monitor/events?${params}`);

  function finish(title, message) {
    source.close();
    heading.textContent = title;
    status.textContent = message;
    stop.disabled = true;
  }

  source.addEventListener("snapshots", (event) => {
    status.textContent = "Monitoring snapshot notifications and metrics…";
    // HTML is rendered and escaped by the server's Askama template.
    results.innerHTML = event.data;
  });
  source.addEventListener("complete", (event) => {
    results.innerHTML = event.data;
    finish("Monitoring completed", "The monitoring window has ended.");
  });
  source.addEventListener("monitor-error", (event) => {
    error.textContent = event.data;
    error.hidden = false;
    finish("Monitoring failed", "The notification consumer stopped. Start a new session to try again.");
  });
  source.addEventListener("error", () => {
    // Do not reconnect automatically: a new connection owns a new session.
    error.textContent = "The notification stream disconnected.";
    error.hidden = false;
    finish("Monitoring disconnected", "Collected snapshot status is retained below.");
  });
  stop.addEventListener("click", () => {
    finish("Monitoring stopped", "Collected snapshot status is retained below.");
  });
  window.addEventListener("pagehide", () => source.close(), { once: true });
})();
