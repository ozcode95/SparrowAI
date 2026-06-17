/**
 * Fluent UI v2 ProgressRing — indeterminate circular spinner. Replaces
 * the previous ASCII glyph spinner with the canonical Windows 11
 * affordance. Sizing is consistent across button / panel contexts.
 */
export function Spinner({ size = "md" }: { size?: "sm" | "md" | "lg" } = {}) {
  const cls =
    size === "sm"
      ? "fluent-ring fluent-ring-sm"
      : size === "lg"
        ? "fluent-ring fluent-ring-lg"
        : "fluent-ring";
  return <span aria-label="loading" role="status" className={cls} />;
}
