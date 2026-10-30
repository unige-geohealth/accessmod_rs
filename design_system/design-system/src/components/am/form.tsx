import { useId, type ReactNode } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { type Key, T, useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

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

export function RadioField({
  label,
  options,
  value,
  onChange,
}: {
  label: Key;
  options: { value: string; label: Key }[];
  value: string;
  onChange?: (v: string) => void;
}) {
  const id = useId();
  return (
    <div role="group" aria-labelledby={id} className="flex flex-col gap-[5px]">
      <span id={id} className="font-bold">
        <T k={label} />
      </span>
      <RadioGroup value={value} onValueChange={onChange} className="gap-1">
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
}: {
  label: Key;
  help?: Key;
  checked: boolean;
  onChange?: (v: boolean) => void;
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <label className="flex items-center gap-2">
        <Checkbox checked={checked} onCheckedChange={(v) => onChange?.(v === true)} />
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
