import { useQuery } from "@tanstack/react-query";

export function About() {
  const health = useQuery({
    queryKey: ["health"],
    queryFn: async () => (await fetch("/api/health")).json() as Promise<{ version: string }>,
  });
  return (
    <section>
      <h1>AccessMod</h1>
      <p>Physical accessibility to health services: travel time, coverage, referral.</p>
      <p className="muted">Engine {health.data?.version ?? "…"} · LGPL-3.0-or-later</p>
    </section>
  );
}
