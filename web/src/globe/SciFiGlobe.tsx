import { useEffect, useRef } from "react";
import createGlobe, { type Marker } from "cobe";
import type { GlobeMetric, Region } from "../api/types";

// Same ramp `App.tsx`'s `heatColor()` uses for the region × hour heatmap —
// duplicated (not imported) so this component has no dependency on
// `App.tsx`, but kept visually identical on purpose. If one changes, change
// both.
const HEAT_RAMP: Array<[number, number, number]> = [
  [0x17 / 255, 0x32 / 255, 0x3c / 255],
  [0x1d / 255, 0x59 / 255, 0x60 / 255],
  [0x2e / 255, 0x88 / 255, 0x7c / 255],
  [0x6d / 255, 0xb2 / 255, 0x69 / 255],
  [0xc9 / 255, 0xb2 / 255, 0x4a / 255],
  [0xe8 / 255, 0x8f / 255, 0x52 / 255],
  [0xf0 / 255, 0x70 / 255, 0x8a / 255],
];

function heatColorRgb(value: number): [number, number, number] {
  return HEAT_RAMP[Math.min(HEAT_RAMP.length - 1, Math.max(0, Math.floor(value * HEAT_RAMP.length)))];
}

function metricValue(region: Region, metric: GlobeMetric): number | null {
  if (metric === "failure") return region.failure;
  if (metric === "latency") return region.p95;
  return region.volume;
}

const NO_SIGNAL: [number, number, number] = [0.2, 0.16, 0.32]; // dim violet — "signal lost," never a cold success color

function toVector3(latitude: number, longitude: number): [number, number, number] {
  const lat = (latitude * Math.PI) / 180;
  const lon = (longitude * Math.PI) / 180;
  return [Math.cos(lat) * Math.sin(lon), Math.sin(lat), Math.cos(lat) * Math.cos(lon)];
}

/** Standard phi (yaw, around Y)-then-theta (pitch, around X) rotation — the same two-angle convention COBE's own `phi`/`theta` options use for camera orientation. Used only for click hit-testing (COBE v2 has no built-in marker hit-testing), so a close approximation is enough: it drives which marker a click resolves to, not anything rendered. */
function projected(region: Region, phi: number, theta: number): { x: number; y: number; visible: boolean } {
  let [x, y, z] = toVector3(region.latitude, region.longitude);
  const cosPhi = Math.cos(phi);
  const sinPhi = Math.sin(phi);
  [x, z] = [x * cosPhi - z * sinPhi, x * sinPhi + z * cosPhi];
  const cosTheta = Math.cos(theta);
  const sinTheta = Math.sin(theta);
  [y, z] = [y * cosTheta - z * sinTheta, y * sinTheta + z * cosTheta];
  return { x, y, visible: z > -0.15 };
}

export function SciFiGlobe({
  regions,
  metric,
  selected,
  onSelect,
}: {
  regions: Region[];
  metric: GlobeMetric;
  selected: string;
  onSelect: (code: string) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const phiRef = useRef(0.4);
  const thetaRef = useRef(0.15);
  const pointerRef = useRef<{ x: number; y: number; dragged: boolean } | null>(null);
  // Re-read on every render rather than captured once — markers must follow
  // `metric`/`selected` changes without tearing down the WebGL context.
  const regionsRef = useRef(regions);
  const metricRef = useRef(metric);
  const selectedRef = useRef(selected);
  regionsRef.current = regions;
  metricRef.current = metric;
  selectedRef.current = selected;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const maxVolume = Math.max(...regions.map((r) => r.volume ?? 0), 1);
    const maxMetric = Math.max(...regions.map((r) => metricValue(r, metric) ?? 0), 1);

    const buildMarkers = (): Marker[] =>
      regionsRef.current.map((region) => {
        const value = metricValue(region, metricRef.current);
        const hasSignal = region.status !== "unknown" && region.status !== "pending" && value !== null;
        const baseSize = region.volume ? 0.035 + Math.sqrt(region.volume / maxVolume) * 0.065 : 0.04;
        return {
          location: [region.latitude, region.longitude],
          size: region.code === selectedRef.current ? baseSize * 1.35 : baseSize,
          color: hasSignal ? heatColorRgb((value ?? 0) / maxMetric) : NO_SIGNAL,
          id: region.code,
        };
      });

    const size = canvas.clientWidth || 320;
    const dpr = window.devicePixelRatio || 1;

    const globe = createGlobe(canvas, {
      width: size,
      height: size,
      phi: phiRef.current,
      theta: thetaRef.current,
      dark: 1,
      diffuse: 1.6,
      scale: 1.05,
      devicePixelRatio: dpr,
      mapSamples: 14000,
      mapBrightness: 5.5,
      baseColor: [0.05, 0.14, 0.17],
      markerColor: [0.33, 0.84, 0.91],
      glowColor: [0.22, 0.55, 0.6],
      offset: [0, 0],
      markers: buildMarkers(),
    });

    let frame = 0;
    const render = () => {
      if (!pointerRef.current) {
        phiRef.current += 0.0022;
      }
      globe.update({
        phi: phiRef.current,
        theta: thetaRef.current,
        markers: buildMarkers(),
      });
      frame = requestAnimationFrame(render);
    };
    frame = requestAnimationFrame(render);

    const onResize = () => {
      const next = canvas.clientWidth || size;
      globe.update({ width: next, height: next });
    };
    window.addEventListener("resize", onResize);

    const pickMarker = (clientX: number, clientY: number): string | null => {
      const rect = canvas.getBoundingClientRect();
      const nx = ((clientX - rect.left) / rect.width) * 2 - 1;
      const ny = ((clientY - rect.top) / rect.height) * 2 - 1;
      let closest: { code: string; dist: number } | null = null;
      for (const region of regionsRef.current) {
        const p = projected(region, phiRef.current, thetaRef.current);
        if (!p.visible) continue;
        const dist = Math.hypot(p.x - nx, -p.y - ny);
        if (dist < 0.09 && (!closest || dist < closest.dist)) closest = { code: region.code, dist };
      }
      return closest?.code ?? null;
    };

    const onPointerDown = (event: PointerEvent) => {
      pointerRef.current = { x: event.clientX, y: event.clientY, dragged: false };
      canvas.setPointerCapture(event.pointerId);
    };
    const onPointerMove = (event: PointerEvent) => {
      const pointer = pointerRef.current;
      if (!pointer) return;
      const dx = event.clientX - pointer.x;
      const dy = event.clientY - pointer.y;
      if (Math.abs(dx) > 2 || Math.abs(dy) > 2) pointer.dragged = true;
      phiRef.current += dx * 0.005;
      thetaRef.current = Math.max(-1.1, Math.min(1.1, thetaRef.current - dy * 0.005));
      pointer.x = event.clientX;
      pointer.y = event.clientY;
    };
    const onPointerUp = (event: PointerEvent) => {
      const pointer = pointerRef.current;
      pointerRef.current = null;
      if (pointer && !pointer.dragged) {
        const code = pickMarker(event.clientX, event.clientY);
        if (code) onSelect(code);
      }
    };

    canvas.addEventListener("pointerdown", onPointerDown);
    canvas.addEventListener("pointermove", onPointerMove);
    canvas.addEventListener("pointerup", onPointerUp);
    canvas.addEventListener("pointerleave", onPointerUp);

    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", onResize);
      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      canvas.removeEventListener("pointerup", onPointerUp);
      canvas.removeEventListener("pointerleave", onPointerUp);
      globe.destroy();
    };
    // Rebuilding the WebGL context on every `regions`/`metric` change would
    // flash and drop the current rotation — `regionsRef`/`metricRef` carry
    // updates into the running render loop instead. Only mount/unmount
    // recreate the globe.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className={`scifi-globe ${selected ? `scifi-globe-has-selection` : ""}`}>
      <canvas ref={canvasRef} />
    </div>
  );
}
