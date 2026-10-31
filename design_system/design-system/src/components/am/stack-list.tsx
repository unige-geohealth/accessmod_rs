import { type DragEvent, type KeyboardEvent, useState } from "react";
import { type Key, T, useI18n } from "#ui/i18n";
import { cn } from "#ui/lib/utils";
import { icons } from "./icons";

export type StackItem = { id: string; label: string; kind: "raster" | "vector" };
export type Stack = { use: StackItem[]; skip: StackItem[] };
type Side = keyof Stack;

/**
 * AM5 `amDoubleSortableInput` (Merge land cover › Merge): two ordered lists,
 * items dragged within a list (order = merge priority, top wins) or across
 * ("use" / "skip"). Keyboard: Alt+↑/↓ reorders, Enter moves across.
 * Controlled: the parent owns the stack.
 */
export function StackList({
  value,
  onChange,
  labels = { use: "toolbox_land_cover_merge_items_to_use", skip: "toolbox_land_cover_merge_items_to_skip" },
}: {
  value: Stack;
  onChange: (s: Stack) => void;
  labels?: Record<Side, Key>;
}) {
  const { t } = useI18n();
  const [drag, setDrag] = useState<{ side: Side; index: number } | null>(null);
  const [over, setOver] = useState<{ side: Side; index: number } | null>(null);

  const move = (from: { side: Side; index: number }, to: { side: Side; index: number }) => {
    const next: Stack = { use: [...value.use], skip: [...value.skip] };
    const [item] = next[from.side].splice(from.index, 1);
    const at = from.side === to.side && from.index < to.index ? to.index - 1 : to.index;
    if (item) next[to.side].splice(at, 0, item);
    onChange(next);
  };

  return (
    <div className="grid grid-cols-2 gap-[30px]">
      {(["use", "skip"] as const).map((side) => (
        <div key={side}>
          <h3 className="mb-4 text-2xl">
            <T k={labels[side]} />
          </h3>
          <ol
            aria-label={t(labels[side])}
            onDragOver={(e) => {
              e.preventDefault();
              if (!value[side].length) setOver({ side, index: 0 });
            }}
            onDrop={(e) => {
              e.preventDefault();
              if (drag && over) move(drag, over);
              setDrag(null);
              setOver(null);
            }}
            className={cn(
              "min-h-11 border-x border-y-2 border-x-transparent border-y-foreground/80 px-1.5 py-1",
              drag && "border-x-panel-border",
            )}
          >
            {value[side].map((item, index) => (
              <Item
                key={item.id}
                item={item}
                dragging={drag?.side === side && drag.index === index}
                dropBefore={over?.side === side && over.index === index}
                onDragStart={() => setDrag({ side, index })}
                onDragOver={(e: DragEvent) => {
                  e.preventDefault();
                  const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                  setOver({ side, index: e.clientY > r.top + r.height / 2 ? index + 1 : index });
                }}
                onDragEnd={() => {
                  setDrag(null);
                  setOver(null);
                }}
                onKeyDown={(e: KeyboardEvent) => {
                  const other: Side = side === "use" ? "skip" : "use";
                  if (e.key === "Enter") move({ side, index }, { side: other, index: 0 });
                  else if (e.altKey && e.key === "ArrowUp" && index > 0) move({ side, index }, { side, index: index - 1 });
                  else if (e.altKey && e.key === "ArrowDown" && index < value[side].length - 1)
                    move({ side, index }, { side, index: index + 2 });
                  else return;
                  e.preventDefault();
                }}
              />
            ))}
            {!value[side].length && (
              <li className="flex h-10 items-center justify-center text-muted-foreground">
                <T k="stack.empty" />
              </li>
            )}
            {over?.side === side && over.index === value[side].length && value[side].length > 0 && (
              <li aria-hidden className="h-0.5 bg-primary" />
            )}
          </ol>
        </div>
      ))}
    </div>
  );
}

function Item({
  item,
  dragging,
  dropBefore,
  ...handlers
}: {
  item: StackItem;
  dragging: boolean;
  dropBefore: boolean;
  onDragStart: () => void;
  onDragOver: (e: DragEvent) => void;
  onDragEnd: () => void;
  onKeyDown: (e: KeyboardEvent) => void;
}) {
  const { t } = useI18n();
  const Grip = icons.drag;
  const Kind = item.kind === "raster" ? icons.stackRaster : icons.stackVector;
  return (
    <li
      draggable
      tabIndex={0}
      title={t("stack.move")}
      {...handlers}
      className={cn(
        "flex cursor-grab items-center gap-3 border-b border-panel-border bg-background px-3 py-2.5 last:border-b-0",
        "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
        dragging && "opacity-40",
        dropBefore && "shadow-[inset_0_2px_0_var(--primary)]",
      )}
    >
      <Grip className="size-4 shrink-0 text-muted-foreground" />
      <span className="flex-1">{item.label}</span>
      <Kind className="size-4 shrink-0" />
    </li>
  );
}
