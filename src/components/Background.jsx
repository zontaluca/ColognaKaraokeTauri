import { useEffect, useRef } from "react";

/**
 * Lightweight animated backgrounds inspired by ReactBits (no external deps).
 * - Player: Aurora-like gradient drift (canvas 2D).
 * - Library/Leaderboard: Threads — soft diagonal streaks.
 * - Others: static radial gradient (respects prefers-reduced-motion).
 */
export default function Background({ view }) {
  const canvasRef = useRef(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const ctx = canvas.getContext("2d");
    let raf = 0;
    let t = 0;

    const resize = () => {
      canvas.width = window.innerWidth;
      canvas.height = window.innerHeight;
    };
    resize();
    window.addEventListener("resize", resize);

    const drawAurora = () => {
      const { width: w, height: h } = canvas;
      ctx.clearRect(0, 0, w, h);
      const blobs = [
        { x: 0.2 + Math.sin(t * 0.0003) * 0.15, y: 0.3 + Math.cos(t * 0.00025) * 0.2, c: "rgba(255, 77, 157, 0.45)" },
        { x: 0.8 + Math.cos(t * 0.00022) * 0.15, y: 0.7 + Math.sin(t * 0.00028) * 0.2, c: "rgba(77, 157, 255, 0.45)" },
        { x: 0.5 + Math.sin(t * 0.0002) * 0.25, y: 0.5 + Math.cos(t * 0.00035) * 0.25, c: "rgba(179, 102, 255, 0.35)" },
      ];
      for (const b of blobs) {
        const g = ctx.createRadialGradient(b.x * w, b.y * h, 0, b.x * w, b.y * h, Math.max(w, h) * 0.45);
        g.addColorStop(0, b.c);
        g.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = g;
        ctx.fillRect(0, 0, w, h);
      }
    };

    const drawThreads = () => {
      const { width: w, height: h } = canvas;
      ctx.clearRect(0, 0, w, h);
      ctx.globalAlpha = 0.25;
      for (let i = 0; i < 14; i++) {
        const y = (i / 14) * h + Math.sin(t * 0.0005 + i) * 20;
        const grad = ctx.createLinearGradient(0, y, w, y + 30);
        grad.addColorStop(0, "rgba(255,77,157,0.0)");
        grad.addColorStop(0.5, "rgba(77,157,255,0.6)");
        grad.addColorStop(1, "rgba(255,77,157,0.0)");
        ctx.strokeStyle = grad;
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        ctx.moveTo(0, y);
        for (let x = 0; x <= w; x += 20) {
          ctx.lineTo(x, y + Math.sin(t * 0.0008 + x * 0.005 + i) * 14);
        }
        ctx.stroke();
      }
      ctx.globalAlpha = 1;
    };

    const loop = () => {
      if (view === "player") drawAurora();
      else drawThreads();
      t += 16;
      raf = requestAnimationFrame(loop);
    };

    if (reduced) {
      if (view === "player") drawAurora(); else drawThreads();
    } else {
      raf = requestAnimationFrame(loop);
    }

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", resize);
    };
  }, [view]);

  return (
    <canvas
      ref={canvasRef}
      style={{
        position: "fixed",
        inset: 0,
        width: "100vw",
        height: "100vh",
        zIndex: -1,
        opacity: view === "player" ? 0.55 : 0.35,
        pointerEvents: "none",
      }}
      aria-hidden
    />
  );
}
