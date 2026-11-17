import type { Shape } from "./layout";

/** Search in paint order, without copying the shapes on every pointer event. */
export function hitShape(
  shapes: readonly Shape[],
  mode: "treemap" | "sunburst",
  width: number,
  height: number,
  x: number,
  y: number,
): Shape | undefined {
  const radius = Math.hypot(x - width / 2, y - height / 2);
  let angle = Math.atan2(y - height / 2, x - width / 2);
  if (angle < -Math.PI / 2) angle += Math.PI * 2;
  for (let i = shapes.length - 1; i >= 0; i--) {
    const s = shapes[i];
    if (
      mode === "treemap"
        ? x >= s.x && x <= s.x + s.width && y >= s.y && y <= s.y + s.height
        : radius >= s.inner &&
          radius <= s.outer &&
          angle >= s.start &&
          angle <= s.end
    )
      return s;
  }
  return undefined;
}
