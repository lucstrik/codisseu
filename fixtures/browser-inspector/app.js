// Original Codisseu fixture. All tokens below are inert test values.
localStorage.setItem('access_token', 'fixture-local-token-42');
localStorage.setItem('theme', 'light');
localStorage.setItem('jwt_token', 'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJmaXh0dXJlLXVzZXIiLCJhbnN3ZXIiOjQyLCJpc3MiOiJjb2Rpc3NldS1maXh0dXJlIn0.eyJpbnZhbGlkIjoiZml4dHVyZS1vbmx5In0');
sessionStorage.setItem('session_token', 'fixture-session-token-42');
sessionStorage.setItem('counter', '1');

window.computeProbe = function computeProbe(left = 19, right = 23) {
  const result = left + right;
  debugger;
  const doubled = result * 2;
  window.lastResult = result;
  document.getElementById('answer').textContent = String(result);
  document.title = 'InspectorCompute:' + result;
  return doubled;
};

window.sendProbe = async function sendProbe() {
  const response = await fetch('/api/echo', {
    method: 'POST',
    headers: {
      'Authorization': 'Bearer fixture-bearer-42',
      'Content-Type': 'application/json',
      'X-Fixture': 'request42',
    },
    body: JSON.stringify({ left: 19, right: 23, message: 'ação😀' }),
  });
  const value = await response.json();
  window.lastResponse = value;
  document.getElementById('status').textContent = JSON.stringify({ status: response.status, ...value });
  document.title = 'InspectorNetwork:' + value.answer;
  return value;
};

document.getElementById('request').addEventListener('click', () => sendProbe());
document.getElementById('compute').addEventListener('click', () => computeProbe());
document.getElementById('storage-check').addEventListener('click', () => {
  const value = {
    local: Object.fromEntries(Object.entries(localStorage)),
    session: Object.fromEntries(Object.entries(sessionStorage)),
    visibleCookies: document.cookie,
  };
  document.getElementById('status').textContent = JSON.stringify(value);
  window.lastStorageCheck = value;
});

customElements.define('fixture-counter', class extends HTMLElement {
  constructor() {
    super();
    const shadow = this.attachShadow({ mode: 'open' });
    shadow.innerHTML = '<style>button{padding:6px 10px;font:inherit}span{margin-left:8px}</style><button id="shadow-compute">Somar no shadow42</button><span id="shadow-result">WebComponent real</span>';
    shadow.getElementById('shadow-compute').addEventListener('click', () => {
      const result = computeProbe();
      shadow.getElementById('shadow-result').textContent = String(result);
    });
  }
});

// A real periodic JS task lets the native Pause control interrupt an otherwise
// idle fixture. It performs no network request and holds no external state.
setInterval(function fixtureHeartbeat() {
  window.fixtureTicks = (window.fixtureTicks || 0) + 1;
}, 50);
