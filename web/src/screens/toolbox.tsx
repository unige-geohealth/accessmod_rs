// Toolbox (AM5 `amToolbox`): the demo project tool; the AM5 tools come in a
// later milestone.
import { Columns } from "@accessmod/ui/am/columns";
import { ConfirmDialog } from "@accessmod/ui/am/confirm-dialog";
import { AmButton } from "@accessmod/ui/am/messages";
import { PanelAccordion } from "@accessmod/ui/am/panel-accordion";
import { T } from "@accessmod/ui/i18n";
import { useState } from "react";
import { api } from "../api/client";
import { keys, queryClient } from "../api/queries";
import { HOME, navigate, type Route } from "../app/route";
import { useBatch } from "../state/batch";
import { BatchStatus } from "./common";
import { NotYet } from "./other";

export function ToolboxScreen({ route }: { route: Route }) {
  return (
    <Columns
      left={
        <PanelAccordion
          value={route.panel ?? ""}
          onValueChange={(v) => navigate({ ...route, panel: v === "" ? null : v, query: {} })}
          items={[{ value: "demo", icon: "reset", title: "toolbox.demo_title", content: <DemoPanel /> }]}
        />
      }
      right={<NotYet />}
    />
  );
}

function DemoPanel() {
  const [confirming, setConfirming] = useState(false);
  const { batch, start } = useBatch();
  const reset = async () => {
    const built: { slug?: string } = {};
    await start(
      ["Demo"],
      (name) => name,
      async () => {
        built.slug = (await api.command("demo.reset", {})).slug;
      },
    );
    await queryClient.invalidateQueries({ queryKey: keys.projects });
    if (built.slug !== undefined) {
      await queryClient.invalidateQueries({ queryKey: ["project", built.slug] });
      navigate({ ...HOME, project: built.slug });
    }
  };
  return (
    <div className="flex flex-col gap-[15px]">
      <p>
        <T k="toolbox.demo_text" />
      </p>
      <AmButton variant="danger" block icon="reset" k="toolbox.demo_reset" disabled={batch.state === "running"} onClick={() => setConfirming(true)} />
      <BatchStatus batch={batch} doneKey="toolbox.demo_done" />
      <ConfirmDialog open={confirming} onOpenChange={setConfirming} confirm="toolbox.demo_reset" onConfirm={() => void reset()}>
        <T k="toolbox.demo_reset_confirm" />
      </ConfirmDialog>
    </div>
  );
}
