import { type KeyboardEvent, useId, useState, type ReactNode } from "react";
import { Checkbox } from "#ui/components/ui/checkbox";
import { Input } from "#ui/components/ui/input";
import { RadioGroup, RadioGroupItem } from "#ui/components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "#ui/components/ui/select";
import { type Key, T, useI18n } from "#ui/i18n";
import { cn } from "#ui/lib/utils";

/**
 * AM5 form row (`.form-group`): bold label above a full-width control,
 * optional muted help below. `indent` reproduces AM5 dependent fields
 * (e.g. facility ID / name / capacity under the facility layer).
 */
export function Field({
  label,
  help,
  indent,
  htmlFor,
  helpAbove,
  children,
}: {
  label?: Key;
  help?: Key;
  indent?: boolean;
  htmlFor?: string;
  /** AM5 text inputs show the help between label and input. */
  helpAbove?: boolean;
  children: ReactNode;
}) {
  const helpText = help && (
    <p className={cn("text-xs text-muted-foreground", helpAbove && "font-bold")}>
      <T k={help} />
    </p>
  );
  return (
    <div className={cn("flex flex-col gap-[5px]", indent && "pl-12")}>
      {label && (
        <label htmlFor={htmlFor} className="font-bold">
          <T k={label} />
        </label>
      )}
      {helpAbove && helpText}
      {children}
      {!helpAbove && helpText}
    </div>
  );
}

export type Option = { value: string; label: string };

/** Data selectors read "(r) population [demo patients]": type, class, tags. */
export function dataLabel(type: "r" | "v" | "t", cls: string, tags: string) {
  return `(${type}) ${cls} [${tags}]`;
}

export function SelectField({
  label,
  options,
  value,
  onChange,
  indent,
  placeholder = "placeholder_enter_value",
}: {
  label: Key;
  options: Option[];
  value?: string;
  onChange?: (v: string) => void;
  indent?: boolean;
  placeholder?: Key;
}) {
  const id = useId();
  const { t } = useI18n();
  return (
    <Field label={label} indent={indent} htmlFor={id}>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id} className="w-full bg-input-background">
          <SelectValue placeholder={options.length ? t(placeholder) : t("no_data")} />
        </SelectTrigger>
        <SelectContent>
          {options.map((o) => (
            <SelectItem key={o.value} value={o.value}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Field>
  );
}

export function TextField({
  label,
  help,
  value,
  onChange,
  type = "text",
}: {
  label: Key;
  help?: Key;
  value?: string | number;
  onChange?: (v: string) => void;
  type?: "text" | "number";
}) {
  const id = useId();
  return (
    <Field label={label} help={help} htmlFor={id} helpAbove>
      <Input
        id={id}
        type={type}
        value={value}
        onChange={(e) => onChange?.(e.target.value)}
        className="bg-input-background"
      />
    </Field>
  );
}

/** A suggestion of a [`ComboboxField`]: the value it fills in, and a muted note. */
export interface Suggestion {
  value: string;
  note?: string;
}

/**
 * A text field with suggestions below it while focused (ARIA combobox):
 * free text is always allowed, the list only helps. Picking a suggestion
 * fills the field and keeps the focus, so it can be edited at once (e.g.
 * pick the tags `test 4`, then type `5`). Arrow keys move, Enter picks,
 * Escape closes.
 */
export function ComboboxField({
  label,
  help,
  value,
  onChange,
  suggestions,
}: {
  label: Key;
  help?: Key;
  value: string;
  onChange: (v: string) => void;
  suggestions: readonly Suggestion[];
}) {
  const id = useId();
  const list = `${id}-list`;
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const shown = open && suggestions.length > 0;
  const at = Math.min(active, suggestions.length - 1);
  const pick = (s: Suggestion) => {
    onChange(s.value);
    setOpen(false);
    setActive(-1);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setOpen(true);
      setActive(Math.min(at + 1, suggestions.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive(Math.max(at - 1, -1));
    } else if (e.key === "Enter" && shown && suggestions[at] !== undefined) {
      e.preventDefault();
      pick(suggestions[at]);
    } else if (e.key === "Escape" && shown) {
      e.preventDefault();
      setOpen(false);
    }
  };
  return (
    <Field label={label} help={help} htmlFor={id} helpAbove>
      <div className="relative">
        <Input
          id={id}
          role="combobox"
          aria-expanded={shown}
          aria-controls={list}
          aria-autocomplete="list"
          aria-activedescendant={shown && at >= 0 ? `${list}-${at}` : undefined}
          autoComplete="off"
          value={value}
          onChange={(e) => {
            onChange(e.target.value);
            setOpen(true);
            setActive(-1);
          }}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={onKeyDown}
          className="bg-input-background"
        />
        {shown && (
          <ul id={list} role="listbox" className="absolute top-full right-0 left-0 z-20 mt-0.5 max-h-60 overflow-y-auto border border-panel-border bg-background text-sm shadow-md">
            {suggestions.map((s, i) => (
              <li
                key={s.value}
                id={`${list}-${i}`}
                role="option"
                aria-selected={i === at}
                // Before the input's blur: the click picks.
                onMouseDown={(e) => {
                  e.preventDefault();
                  pick(s);
                }}
                className={cn("flex cursor-pointer justify-between gap-3 px-2 py-1 hover:bg-accent", i === at && "bg-accent")}
              >
                <span className="break-all">{s.value}</span>
                {s.note && <span className="shrink-0 text-muted-foreground">{s.note}</span>}
              </li>
            ))}
          </ul>
        )}
      </div>
    </Field>
  );
}

export function RadioField({
  label,
  options,
  value,
  onChange,
  inline,
}: {
  label: Key;
  options: { value: string; label: Key }[];
  value: string;
  onChange?: (v: string) => void;
  /** Options on one line (AM5 `radio-inline`, e.g. Data › Filter). */
  inline?: boolean;
}) {
  const id = useId();
  return (
    <div role="group" aria-labelledby={id} className="flex flex-col gap-[5px]">
      <span id={id} className="font-bold">
        <T k={label} />
      </span>
      <RadioGroup value={value} onValueChange={onChange} className={cn("gap-1", inline && "flex flex-wrap gap-x-5")}>
        {options.map((o) => (
          <label key={o.value} className="flex items-center gap-2">
            <RadioGroupItem value={o.value} />
            <T k={o.label} />
          </label>
        ))}
      </RadioGroup>
    </div>
  );
}

export function CheckboxField({
  label,
  help,
  checked,
  onChange,
  disabled,
}: {
  label: Key;
  help?: Key;
  checked: boolean;
  onChange?: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <label className="flex items-center gap-2">
        <Checkbox checked={checked} disabled={disabled} onCheckedChange={(v) => onChange?.(v === true)} />
        <T k={label} />
      </label>
      {help && (
        <p className="pl-6 text-xs text-muted-foreground">
          <T k={help} />
        </p>
      )}
    </div>
  );
}
