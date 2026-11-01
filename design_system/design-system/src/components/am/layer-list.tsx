import { type DragEvent, type KeyboardEvent, useId, useState } from "react";
import { Input } from "#ui/components/ui/input";
import { T, useI18n } from "#ui/i18n";
import { cn } from "#ui/lib/utils";
import { type IconName, icons } from "./icons";

/** Geometry of a map layer, shown as a compact icon. */
export type LayerKind = "point" | "line" | "polygon" | "raster";

export type LayerOption = { id: string; label: string; kind: LayerKind };
export type LayerItem = LayerOption & { visible: boolean };

const KIND_ICON: Record<LayerKind, IconName> = {
  point: "geometryPoint",
  line: "geometryLine",
  polygon: "geometryPolygon",
  raster: "stackRaster",
};

/** Options listed at most; the search narrows hundreds of layers down. */
const SHOWN = 50;

const fold = (s: string) => s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

function KindIcon({ kind }: { kind: LayerKind }) {
  const { t } = useI18n();
  const Icon = icons[KIND_ICON[kind]];
  return <Icon className="size-4 shrink-0 text-muted-foreground" aria-label={t(`layers.kind_${kind}`)} role="img" />;
}

/**
 * A search field that lists the matching layers below it (a combobox):
 * typing filters, ↑/↓ and Enter or a click add one. For hundreds of
 * layers; the active list is a {@link LayerList}.
 */
export function LayerPicker({ options, onAdd, disabled }: { options: LayerOption[]; onAdd: (id: string) => void; disabled?: boolean }) {
  const { t } = useI18n();
  const list = useId();
  const [text, setText] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const words = fold(text).split(/\s+/).filter(Boolean);
  const matches = options.filter((o) => words.every((w) => fold(o.label).includes(w)));
  const shown = matches.slice(0, SHOWN);
  const SearchIcon = icons.search;
  // A choice closes the list; typing or ↓ opens it again.
  const choose = (option: LayerOption | undefined) => {
    if (!option) return;
    onAdd(option.id);
    setText("");
    setActive(0);
    setOpen(false);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      setActive((a) => (open ? Math.min(a + 1, shown.length - 1) : 0));
      setOpen(true);
    } else if (e.key === "ArrowUp") setActive((a) => Math.max(a - 1, 0));
    else if (e.key === "Enter") choose(shown[active]);
    else if (e.key === "Escape") setOpen(false);
    else return;
    e.preventDefault();
  };
  return (
    <div className="relative">
      <SearchIcon className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        role="combobox"
        aria-expanded={open}
        aria-controls={list}
        aria-label={t("layers.search")}
        placeholder={t("layers.search")}
        disabled={disabled}
        value={text}
        className="pl-9"
        onChange={(e) => {
          setText(e.target.value);
          setActive(0);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onKeyDown={onKeyDown}
      />
      {open && !disabled && (
        <ul
          id={list}
          role="listbox"
          aria-label={t("layers.search")}
          className="absolute z-20 mt-1 max-h-72 w-full overflow-auto rounded-md border border-input bg-background py-1 shadow-md"
        >
          {shown.map((o, i) => (
            <li
              key={o.id}
              role="option"
              aria-selected={i === active}
              // Before the input's blur closes the list.
              onMouseDown={(e) => {
                e.preventDefault();
                choose(o);
              }}
              onMouseEnter={() => setActive(i)}
              className={cn("flex cursor-pointer items-center gap-2.5 px-3 py-1.5", i === active && "bg-accent")}
            >
              <KindIcon kind={o.kind} />
              <span className="truncate">{o.label}</span>
            </li>
          ))}
          {matches.length === 0 && (
            <li className="px-3 py-1.5 text-muted-foreground">
              <T k="layers.no_match" />
            </li>
          )}
          {matches.length > SHOWN && (
            <li className="px-3 py-1.5 text-xs text-muted-foreground">{t("layers.more").replace("%s", String(matches.length - SHOWN))}</li>
          )}
        </ul>
      )}
    </div>
  );
}

/**
 * The active layers, top first: show or hide, remove, geometry, and a
 * handle to reorder by drag (Alt+↑/↓ from the keyboard). Controlled: the
 * owner keeps the list.
 */
export function LayerList({
  items,
  onToggle,
  onRemove,
  onOrder,
}: {
  items: LayerItem[];
  onToggle: (id: string) => void;
  onRemove: (id: string) => void;
  onOrder: (ids: string[]) => void;
}) {
  const { t } = useI18n();
  const [drag, setDrag] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);
  const move = (from: number, to: number) => {
    const ids = items.map((i) => i.id);
    const [id] = ids.splice(from, 1);
    if (id === undefined) return;
    ids.splice(from < to ? to - 1 : to, 0, id);
    onOrder(ids);
  };
  if (items.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        <T k="layers.empty" />
      </p>
    );
  }
  const Grip = icons.drag;
  const Delete = icons.delete;
  return (
    <ol aria-label={t("layers.active")} className="flex flex-col gap-1">
      {items.map((item, index) => {
        const Eye = item.visible ? icons.visible : icons.hidden;
        return (
          <li
            key={item.id}
            draggable
            tabIndex={0}
            onDragStart={() => setDrag(index)}
            onDragOver={(e: DragEvent) => {
              e.preventDefault();
              const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
              setOver(e.clientY > r.top + r.height / 2 ? index + 1 : index);
            }}
            onDrop={(e) => {
              e.preventDefault();
              if (drag !== null && over !== null) move(drag, over);
              setDrag(null);
              setOver(null);
            }}
            onDragEnd={() => {
              setDrag(null);
              setOver(null);
            }}
            onKeyDown={(e: KeyboardEvent) => {
              if (e.altKey && e.key === "ArrowUp" && index > 0) move(index, index - 1);
              else if (e.altKey && e.key === "ArrowDown" && index < items.length - 1) move(index, index + 2);
              else return;
              e.preventDefault();
            }}
            className={cn(
              "flex items-center gap-1.5 rounded-md border border-panel-border bg-background px-1.5 py-1",
              "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
              drag === index && "opacity-40",
              over === index && "shadow-[inset_0_2px_0_var(--primary)]",
              over === items.length && index === items.length - 1 && "shadow-[inset_0_-2px_0_var(--primary)]",
            )}
          >
            <IconButton label={t(item.visible ? "layers.hide" : "layers.show")} pressed={item.visible} onClick={() => onToggle(item.id)}>
              <Eye className="size-4" />
            </IconButton>
            <IconButton label={t("layers.remove")} onClick={() => onRemove(item.id)}>
              <Delete className="size-4" />
            </IconButton>
            <span className={cn("flex-1 truncate", !item.visible && "text-muted-foreground")} title={item.label}>
              {item.label}
            </span>
            <KindIcon kind={item.kind} />
            <Grip className="size-4 shrink-0 cursor-grab text-muted-foreground" aria-hidden />
          </li>
        );
      })}
    </ol>
  );
}

function IconButton({ label, pressed, onClick, children }: { label: string; pressed?: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      onClick={onClick}
      className="flex size-7 shrink-0 items-center justify-center rounded text-foreground hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring"
    >
      {children}
    </button>
  );
}
