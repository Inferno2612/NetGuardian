// Лёгкий canvas-график скорости (загрузка/отдача), без внешних библиотек —
// держим CSP простым (script-src 'self') и бандл маленьким.

class SpeedChart {
  constructor(canvas) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d");
    this.down = [];
    this.up = [];
    this._resize();
    window.addEventListener("resize", () => this._resize());
  }

  _resize() {
    const dpr = window.devicePixelRatio || 1;
    const rect = this.canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return; // вкладка скрыта
    this.canvas.width = Math.round(rect.width * dpr);
    this.canvas.height = Math.round(rect.height * dpr);
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.w = rect.width;
    this.h = rect.height;
    this.draw();
  }

  setData(downSeries, upSeries) {
    this.down = downSeries;
    this.up = upSeries;
    this.draw();
  }

  draw() {
    const { ctx, w, h } = this;
    if (!w || !h) return;
    ctx.clearRect(0, 0, w, h);

    // Горизонтальная сетка
    ctx.strokeStyle = "rgba(255,255,255,0.06)";
    ctx.lineWidth = 1;
    for (let i = 1; i < 4; i++) {
      const y = Math.round((h / 4) * i) + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
    }

    if (this.down.length < 2) return;

    const max = Math.max(1, ...this.down, ...this.up) * 1.2;
    const n = this.down.length;
    const stepX = w / (n - 1);
    const pad = 6;
    const pts = (series) =>
      series.map((v, i) => [i * stepX, h - pad - (v / max) * (h - pad * 2)]);

    const trace = (p) => {
      ctx.moveTo(p[0][0], p[0][1]);
      for (let i = 1; i < p.length; i++) {
        const mx = (p[i - 1][0] + p[i][0]) / 2;
        const my = (p[i - 1][1] + p[i][1]) / 2;
        ctx.quadraticCurveTo(p[i - 1][0], p[i - 1][1], mx, my);
      }
      ctx.lineTo(p[p.length - 1][0], p[p.length - 1][1]);
    };

    const draw = (series, rgb) => {
      const p = pts(series);
      const grad = ctx.createLinearGradient(0, 0, 0, h);
      grad.addColorStop(0, `rgba(${rgb},0.30)`);
      grad.addColorStop(1, `rgba(${rgb},0)`);

      ctx.beginPath();
      trace(p);
      ctx.lineTo(w, h);
      ctx.lineTo(0, h);
      ctx.closePath();
      ctx.fillStyle = grad;
      ctx.fill();

      ctx.beginPath();
      trace(p);
      ctx.strokeStyle = `rgb(${rgb})`;
      ctx.lineWidth = 2;
      ctx.lineJoin = "round";
      ctx.stroke();
    };

    draw(this.up, "77,225,182");
    draw(this.down, "110,168,255");
  }
}

window.SpeedChart = SpeedChart;
