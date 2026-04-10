// Debug viewer for egg-stitch SMC runs.
// Loads a debug JSON file and provides step-by-step navigation of all particles.

const $ = id => document.getElementById(id);
const loading = $('loading');
const controls = $('controls');
const slider = $('slider');
const stepLabel = $('stepLabel');
const summary = $('summary');
const tbody = document.querySelector('#ptable tbody');
const resampleInfo = $('resampleInfo');
const chkSort = $('chkSort');
const chkHideDead = $('chkHideDead');

let data = null;   // DebugLog
let step = 0;      // current step index
let tableSortKey = null;
let tableSortAsc = false;

// --- Load ---

const params = new URLSearchParams(location.search);
const file = params.get('file');

if (!file) {
  loading.innerHTML = '<span class="err">no ?file= parameter</span>';
} else {
  fetch('results/' + file)
    .then(r => { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(d => { data = d; init(); })
    .catch(e => { loading.innerHTML = `<span class="err">failed to load ${file}: ${e}</span>`; });
}

// --- Init ---

function init() {
  loading.style.display = 'none';
  controls.style.display = 'flex';
  $('title').textContent = `debug: ${file.replace(/_debug\.json$/, '')}`;
  $('meta').textContent = `${data.num_particles} particles, ${data.steps.length} steps, temp=${data.temperature}`;
  slider.max = data.steps.length - 1;
  slider.value = 0;
  drawBestChart();
  renderStep();
}

// --- Navigation ---

slider.addEventListener('input', () => { step = +slider.value; renderStep(); });
$('btnFirst').addEventListener('click', () => { step = 0; slider.value = 0; renderStep(); });
$('btnLast').addEventListener('click', () => { step = data.steps.length - 1; slider.value = step; renderStep(); });
$('btnPrev').addEventListener('click', () => { if (step > 0) { step--; slider.value = step; renderStep(); } });
$('btnNext').addEventListener('click', () => { if (step < data.steps.length - 1) { step++; slider.value = step; renderStep(); } });
chkSort.addEventListener('change', renderStep);
chkHideDead.addEventListener('change', renderStep);

document.addEventListener('keydown', e => {
  if (e.key === 'ArrowLeft' || e.key === 'a') { $('btnPrev').click(); e.preventDefault(); }
  if (e.key === 'ArrowRight' || e.key === 'd') { $('btnNext').click(); e.preventDefault(); }
  if (e.key === 'Home') { $('btnFirst').click(); e.preventDefault(); }
  if (e.key === 'End') { $('btnLast').click(); e.preventDefault(); }
});

// Column sort
document.querySelectorAll('#ptable th[data-k]').forEach(th => {
  th.addEventListener('click', () => {
    const k = th.dataset.k;
    if (tableSortKey === k) tableSortAsc = !tableSortAsc;
    else { tableSortKey = k; tableSortAsc = false; }
    // uncheck "sort by weight" if user manually sorts
    chkSort.checked = false;
    renderStep();
  });
});

// --- Render ---

function renderStep() {
  if (!data) return;
  const s = data.steps[step];
  stepLabel.textContent = `Step ${s.step}`;
  $('btnPrev').disabled = step === 0;
  $('btnNext').disabled = step === data.steps.length - 1;

  // Summary
  const alive = s.particles.filter(p => p.weight > 0).length;
  const bestCost = s.best_cost;
  const ratio = bestCost ? (data.original_size / bestCost).toFixed(3) : '?';
  summary.innerHTML = `
    <span>alive: <b>${alive}</b> / ${s.particles.length}</span>
    <span>best cost: <span class="best">${bestCost != null ? bestCost.toLocaleString() : '?'}</span></span>
    <span>ratio: <span class="best">${ratio}x</span></span>
    <span>best pattern: <b style="color:#0ea5e9; font-family:monospace">${esc(s.best_pattern || '?')}</b></span>
  `;

  // Count how many times each index was resampled
  const resampleCount = new Map();
  for (const idx of s.resample_indices) {
    resampleCount.set(idx, (resampleCount.get(idx) || 0) + 1);
  }

  // Build augmented particle list
  let particles = s.particles.map((p, i) => ({
    ...p,
    idx: i,
    resample_count: resampleCount.get(i) || 0,
  }));

  // Sorting
  if (chkSort.checked) {
    particles.sort((a, b) => b.weight - a.weight || a.cost - b.cost);
  } else if (tableSortKey) {
    particles.sort((a, b) => {
      const x = a[tableSortKey], y = b[tableSortKey];
      if (x === y) return 0;
      const cmp = x < y ? -1 : 1;
      return tableSortAsc ? cmp : -cmp;
    });
  }

  // Hide dead
  if (chkHideDead.checked) {
    particles = particles.filter(p => p.weight > 0);
  }

  // Max weight for bar scaling
  const maxW = Math.max(1e-12, ...particles.map(p => p.weight));

  tbody.innerHTML = '';
  for (const p of particles) {
    const tr = document.createElement('tr');
    if (p.weight === 0) tr.className = 'dead';
    else if (bestCost != null && p.cost === bestCost) tr.className = 'best-row';
    if (p.resample_count > 0 && p.weight > 0) tr.classList.add('resampled');

    const barW = Math.round(80 * p.weight / maxW);
    tr.innerHTML = `
      <td>${p.idx}</td>
      <td class="pattern" title="${esc(p.pattern)}">${esc(p.pattern)}</td>
      <td>${p.arity}</td>
      <td>${p.cost.toLocaleString()}</td>
      <td><span class="wbar" style="width:${barW}px"></span> ${p.weight > 0 ? (p.weight * 100).toFixed(2) + '%' : '0'}</td>
      <td>${p.num_matches.toLocaleString()}</td>
      <td>${p.resample_count > 0 ? `<b>${p.resample_count}x</b>` : ''}</td>
    `;
    tbody.appendChild(tr);
  }

  // Resample summary
  const unique = new Set(s.resample_indices).size;
  if (s.resample_indices.length > 0) {
    const topResampled = [...resampleCount.entries()]
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([idx, cnt]) => `p${idx} (${cnt}x)`)
      .join(', ');
    resampleInfo.innerHTML = `<b>${unique}</b> unique particles survived out of ${s.particles.length}. Top resampled: ${topResampled}`;
  } else {
    resampleInfo.innerHTML = 'no resampling (run ended this step)';
  }

  // Highlight current step on chart
  highlightStep(step);
}

// --- Best-cost chart ---

function drawBestChart() {
  const canvas = $('bestChart');
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  canvas.width = rect.width * dpr;
  canvas.height = 120 * dpr;
  canvas.style.height = '120px';
  const ctx = canvas.getContext('2d');
  ctx.scale(dpr, dpr);
  const W = rect.width, H = 120;

  const costs = data.steps.map(s => s.best_cost).filter(c => c != null);
  if (costs.length === 0) return;
  const minC = Math.min(...costs);
  const maxC = Math.max(...costs, data.original_size);
  const n = data.steps.length;

  ctx.clearRect(0, 0, W, H);

  // Axes padding
  const pad = { l: 55, r: 10, t: 10, b: 20 };
  const pw = W - pad.l - pad.r;
  const ph = H - pad.t - pad.b;

  // Original size reference line
  const origY = pad.t + ph * (1 - (data.original_size - minC) / (maxC - minC + 1));
  ctx.strokeStyle = '#e5e7eb';
  ctx.lineWidth = 1;
  ctx.setLineDash([4, 4]);
  ctx.beginPath(); ctx.moveTo(pad.l, origY); ctx.lineTo(W - pad.r, origY); ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = '#9ca3af';
  ctx.font = '10px system-ui';
  ctx.textAlign = 'right';
  ctx.fillText('init', pad.l - 4, origY + 3);

  // Best cost line
  ctx.strokeStyle = '#10b981';
  ctx.lineWidth = 2;
  ctx.beginPath();
  let started = false;
  for (let i = 0; i < n; i++) {
    const c = data.steps[i].best_cost;
    if (c == null) continue;
    const x = pad.l + (i / Math.max(1, n - 1)) * pw;
    const y = pad.t + ph * (1 - (c - minC) / (maxC - minC + 1));
    if (!started) { ctx.moveTo(x, y); started = true; } else ctx.lineTo(x, y);
  }
  ctx.stroke();

  // Y-axis labels
  ctx.fillStyle = '#6b7280';
  ctx.font = '10px system-ui';
  ctx.textAlign = 'right';
  ctx.fillText(minC.toLocaleString(), pad.l - 4, pad.t + ph);
  ctx.fillText(maxC.toLocaleString(), pad.l - 4, pad.t + 10);

  // X-axis labels
  ctx.textAlign = 'center';
  ctx.fillText('0', pad.l, H - 4);
  ctx.fillText(String(n - 1), W - pad.r, H - 4);
}

let _highlightLine = null;
function highlightStep(idx) {
  const canvas = $('bestChart');
  const ctx = canvas.getContext('2d');
  const dpr = window.devicePixelRatio || 1;
  const W = canvas.width / dpr, H = 120;
  const pad = { l: 55, r: 10, t: 10, b: 20 };
  const pw = W - pad.l - pad.r;
  const n = data.steps.length;
  // Redraw chart then overlay
  drawBestChart();
  const x = pad.l + (idx / Math.max(1, n - 1)) * pw;
  ctx.save();
  ctx.scale(dpr, dpr);
  ctx.strokeStyle = '#3b82f6';
  ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(x, pad.t); ctx.lineTo(x, H - pad.b); ctx.stroke();
  ctx.restore();
}

function esc(s) { return String(s).replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c])); }
