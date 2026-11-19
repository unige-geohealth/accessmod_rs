// Toolbox (AM5 `amToolbox`): one tab per tool, as AM5 (interactive map,
// merge land cover, adjust population distribution), and the demo project.
// The project tools need an open project; the demo tool does not.
import { Columns } from "@accessmod/ui/am/columns";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { AmButton, Callout } from "@accessmod/ui/am/messages";
import { SectionCard } from "@accessmod/ui/am/section";
import { type SectionTab, SectionTabs } from "@accessmod/ui/am/section-tabs";
import { T } from "@accessmod/ui/i18n";
import { type ReactNode, useState } from "react";
import { api } from "../api/client";
import { keys, queryClient } from "../api/queries";
import { HOME, navigate, type Route } from "../app/route";
import { operationDone } from "../api/tasks";
import { useTransfer } from "../state/transfer";
import { TransferStatus } from "./common";
import { InteractiveMap } from "./toolbox/map";
import { MergeLandCover } from "./toolbox/merge";
import { AdjustPopulation } from "./toolbox/population";

type Tool = "map" | "merge" | "population" | "demo";

const TABS: SectionTab<Tool>[] = [
  { value: "map", label: "tools_map_btn", icon: "map" },
  { value: "merge", label: "tools_merge_lc_btn", icon: "mergeLandCover" },
  { value: "population", label: "tools_correct_pop_btn", icon: "adjustPopulation" },
  { value: "demo", label: "toolbox.demo_title", icon: "reset" },
];

const isTool = (v: string | null): v is Tool => TABS.some((t) => t.value === v);

const TOOLS: Record<Exclude<Tool, "demo">, (props: { project: string; route: Route }) => ReactNode> = {
  map: InteractiveMap,
  merge: MergeLandCover,
  population: AdjustPopulation,
};

export function ToolboxScreen({ route }: { route: Route }) {
  const tool = isTool(route.panel) ? route.panel : route.project === null ? "demo" : "map";
  const Tool = tool === "demo" ? null : TOOLS[tool];
  return (
    <div className="flex flex-col gap-5">
      <SectionTabs tabs={TABS} value={tool} onChange={(v) => navigate({ ...route, panel: v, query: {} })} />
      {Tool === null ? (
        <Columns
          left={
            <SectionCard title="toolbox.demo_title" icon="reset">
              <DemoPanel />
            </SectionCard>
          }
          right={null}
        />
      ) : route.project === null ? (
        <Callout k="project.open_first" />
      ) : (
        <Tool key={route.project} project={route.project} route={route} />
      )}
    </div>
  );
}

function DemoPanel() {
  const [confirming, setConfirming] = useState(false);
  const { state, run } = useTransfer();
  const reset = () =>
    run(async (queued) => {
      const operation = await api.command("demo.reset", {});
      queued({ kind: "operation", id: operation.id });
      const built = await operationDone(operation.id);
      await queryClient.invalidateQueries({ queryKey: keys.projects });
      await queryClient.invalidateQueries({ queryKey: ["project", built.project] });
      navigate({ ...HOME, project: built.project });
      return built.name;
    });
  return (
    <div className="flex flex-col gap-[15px]">
      <p>
        <T k="toolbox.demo_text" />
      </p>
      <AmButton variant="danger" block icon="reset" k="toolbox.demo_reset" disabled={state.state === "processing"} onClick={() => setConfirming(true)} />
      <TransferStatus transfer={state} />
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} confirm="toolbox.demo_reset" onConfirm={() => void reset()}>
        <T k="toolbox.demo_reset_confirm" />
      </ConfirmDialog>
    </div>
  );
}
