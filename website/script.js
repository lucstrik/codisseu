(() => {
  "use strict";

  // The page remains complete without JavaScript. This canvas is decoration only.
  const canvas = document.querySelector("#space-mesh");
  const context = canvas?.getContext("2d");
  if (!context) return;

  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const finePointer = window.matchMedia("(hover: hover) and (pointer: fine)");
  const coordinates = document.querySelector("#coordinates");
  const pointer = { x: -1000, y: -1000, strength: 0 };
  const target = { x: -1000, y: -1000, strength: 0 };
  let width = 0;
  let height = 0;
  let frame = 0;
  let lastTime = 0;
  let spacing = 64;

  function displace(x, y) {
    if (pointer.strength < 0.001) return { x, y };
    const dx = x - pointer.x;
    const dy = y - pointer.y;
    const distance = Math.hypot(dx, dy);
    const force =
      Math.exp(-(distance * distance) / (2 * 145 * 145)) * pointer.strength;
    return { x: x + dx * force * 0.38, y: y + dy * force * 0.38 };
  }

  function draw() {
    context.clearRect(0, 0, width, height);
    context.lineWidth = 0.65;
    context.strokeStyle = "rgba(132, 142, 191, 0.125)";
    context.beginPath();
    const offsetX = (width % spacing) / 2;
    const offsetY = 24;
    const step = spacing / 3;

    // Sampling each line between intersections lets the squared paper bend smoothly.
    for (let x = offsetX - spacing; x <= width + spacing; x += spacing) {
      for (let y = -spacing; y <= height + spacing; y += step) {
        const point = displace(x, y + offsetY);
        if (y === -spacing) context.moveTo(point.x, point.y);
        else context.lineTo(point.x, point.y);
      }
    }
    for (let y = offsetY - spacing; y <= height + spacing; y += spacing) {
      for (let x = -spacing; x <= width + spacing; x += step) {
        const point = displace(x + offsetX, y);
        if (x === -spacing) context.moveTo(point.x, point.y);
        else context.lineTo(point.x, point.y);
      }
    }
    context.stroke();

    if (pointer.strength > 0.005) {
      const glow = context.createRadialGradient(
        pointer.x,
        pointer.y,
        0,
        pointer.x,
        pointer.y,
        240,
      );
      glow.addColorStop(0, `rgba(139, 108, 233, ${0.085 * pointer.strength})`);
      glow.addColorStop(1, "rgba(139, 108, 233, 0)");
      context.fillStyle = glow;
      context.fillRect(0, 0, width, height);
      context.fillStyle = `rgba(197, 178, 255, ${0.38 * pointer.strength})`;
      for (let x = offsetX; x < width; x += spacing) {
        for (let y = offsetY; y < height; y += spacing) {
          if (Math.hypot(x - pointer.x, y - pointer.y) > 170) continue;
          const point = displace(x, y);
          context.fillRect(point.x - 1, point.y - 1, 2, 2);
        }
      }
    }
  }

  function animate(time) {
    frame = 0;
    if (document.hidden || motion.matches || !finePointer.matches) return;
    // Cap drawing to 30fps; stop requesting frames as soon as the spring settles.
    if (time - lastTime < 1000 / 30) {
      frame = requestAnimationFrame(animate);
      return;
    }
    lastTime = time;
    pointer.x += (target.x - pointer.x) * 0.22;
    pointer.y += (target.y - pointer.y) * 0.22;
    pointer.strength += (target.strength - pointer.strength) * 0.16;
    draw();
    const pending =
      Math.abs(pointer.x - target.x) +
      Math.abs(pointer.y - target.y) +
      Math.abs(pointer.strength - target.strength) * 150;
    if (pending > 0.15) frame = requestAnimationFrame(animate);
  }

  function schedule() {
    if (!frame && !document.hidden && !motion.matches && finePointer.matches)
      frame = requestAnimationFrame(animate);
  }

  function resize() {
    width = document.documentElement.clientWidth;
    height = window.innerHeight;
    spacing = width < 760 ? 44 : 64;
    const ratio = Math.min(window.devicePixelRatio || 1, 1.5);
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    draw();
  }

  window.addEventListener(
    "pointermove",
    (event) => {
      if (
        motion.matches ||
        !finePointer.matches ||
        event.pointerType === "touch"
      )
        return;
      if (pointer.x === -1000) {
        pointer.x = event.clientX;
        pointer.y = event.clientY;
      }
      target.x = event.clientX;
      target.y = event.clientY;
      target.strength = 1;
      if (coordinates)
        coordinates.textContent = `X ${(event.clientX / width).toFixed(3)}   Y ${(event.clientY / height).toFixed(3)}`;
      schedule();
    },
    { passive: true },
  );

  function leave() {
    target.strength = 0;
    schedule();
  }
  document.documentElement.addEventListener("pointerleave", leave, {
    passive: true,
  });
  window.addEventListener("blur", leave);
  window.addEventListener("resize", resize, { passive: true });
  document.addEventListener("visibilitychange", () => {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    pointer.strength = target.strength = 0;
    if (!document.hidden) draw();
  });
  function preferenceChanged() {
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    pointer.strength = target.strength = 0;
    if (coordinates) coordinates.textContent = "X 0.000   Y 0.000";
    draw();
  }
  motion.addEventListener("change", preferenceChanged);
  finePointer.addEventListener("change", preferenceChanged);
  resize();
  document.documentElement.classList.add("mesh-ready");
})();
