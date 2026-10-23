// A form generated from a job's JSON Schema. It renders the document and
// emits messages; it holds no state of its own (docs/ui.md, "State").

import type { JobMessage } from "../documents/job";
import type { Api } from "../generated/types";
import {
  datasetClasses,
  defaultValue,
  enumOptions,
  fieldKind,
  getIn,
  items,
  type Json,
  nullable,
  type Path,
  properties,
  resolve,
  type Schema,
  type Variant,
  variantDefault,
  variantOf,
  variants,
} from "./schema";

/** Fields the project fills in: never shown. */
const HIDDEN = new Set(["schema_version", "output_dir"]);

interface Context {
  root: Schema;
  datasets: Api.Dataset[];
  dispatch: (message: JobMessage) => void;
}

/** Classes passed down to the path of a dataset field, and the attribute
 * fields of the dataset chosen there. */
interface Inherited {
  classes: string[] | null;
  fields: string[] | null;
}

const NONE: Inherited = { classes: null, fields: null };

export function SchemaForm({
  root,
  value,
  datasets,
  dispatch,
}: {
  root: Schema;
  value: Json;
  datasets: Api.Dataset[];
  dispatch: (message: JobMessage) => void;
}) {
  const ctx: Context = { root, datasets, dispatch };
  return (
    <div className="form">
      <ObjectFields ctx={ctx} schema={root} value={value} path={[]} inherited={NONE} top />
    </div>
  );
}

/** Identifier of the control editing `path`, for its label. */
const controlId = (path: Path) => `f-${path.join("-")}`;

/** Kinds edited by one control (the others hold sub-fields). */
const CONTROLS = new Set(["string", "number", "integer", "boolean", "enum", "variants"]);

const label = (name: string) => name.replaceAll("_", " ").replace(/^./, (c) => c.toUpperCase());

function description(ctx: Context, schema: Schema): string | undefined {
  const d = resolve(ctx.root, schema).description;
  return typeof d === "string" ? d : undefined;
}

function fieldsOf(ctx: Context, file: Json | undefined): string[] | null {
  if (typeof file !== "string") return null;
  const dataset = ctx.datasets.find((d) => d.file === file);
  return dataset?.summary.kind === "vector" ? dataset.summary.fields : null;
}

function ObjectFields({
  ctx,
  schema,
  value,
  path,
  inherited,
  top = false,
  skip,
}: {
  ctx: Context;
  schema: Schema;
  value: Json | undefined;
  path: Path;
  inherited: Inherited;
  top?: boolean;
  skip?: string;
}) {
  const own = datasetClasses(ctx.root, schema);
  const classes = own ?? inherited.classes;
  // The attribute fields of the dataset this object names.
  const fields = fieldsOf(ctx, getIn(value, ["path"])) ?? inherited.fields;
  const entries = properties(ctx.root, schema).filter(
    ([name]) => !(top && HIDDEN.has(name)) && name !== skip,
  );
  const missing = entries.filter(
    ([name, , required]) => !required && getIn(value, [name]) === undefined,
  );
  return (
    <>
      {entries
        .filter(([name, , required]) => required || getIn(value, [name]) !== undefined)
        .map(([name, property, required]) => (
          <Field
            key={name}
            ctx={ctx}
            name={name}
            schema={property}
            value={getIn(value, [name])}
            path={[...path, name]}
            required={required}
            inherited={{
              classes: name === "path" ? classes : null,
              fields: name.endsWith("_field") ? fields : null,
            }}
          />
        ))}
      {missing.length > 0 && (
        <div className="optional">
          {missing.map(([name, property]) => (
            <button
              type="button"
              key={name}
              className="link"
              title={description(ctx, property)}
              onClick={() =>
                ctx.dispatch({
                  type: "set",
                  path: [...path, name],
                  value: nullable(ctx.root, property)
                    ? defaultValue(ctx.root, nullable(ctx.root, property) ?? property)
                    : defaultValue(ctx.root, property),
                })
              }
            >
              + {label(name)}
            </button>
          ))}
        </div>
      )}
    </>
  );
}

function Field({
  ctx,
  name,
  schema,
  value,
  path,
  required,
  inherited,
}: {
  ctx: Context;
  name: string;
  schema: Schema;
  value: Json | undefined;
  path: Path;
  required: boolean;
  inherited: Inherited;
}) {
  const remove = required ? null : () => ctx.dispatch({ type: "set", path, value: undefined });
  const inner = nullable(ctx.root, schema) ?? schema;
  const controlled = CONTROLS.has(fieldKind(ctx.root, inner));
  return (
    <div className="field">
      <div className="field-label" title={description(ctx, schema)}>
        {controlled ? (
          <label htmlFor={controlId(path)}>{label(name)}</label>
        ) : (
          <span>{label(name)}</span>
        )}
        {remove && (
          <button
            type="button"
            className="link small"
            aria-label={`remove ${label(name)}`}
            onClick={remove}
          >
            remove
          </button>
        )}
      </div>
      <Input ctx={ctx} schema={schema} value={value} path={path} inherited={inherited} />
      {description(ctx, schema) && <div className="hint">{description(ctx, schema)}</div>}
    </div>
  );
}

function Input({
  ctx,
  schema,
  value,
  path,
  inherited,
}: {
  ctx: Context;
  schema: Schema;
  value: Json | undefined;
  path: Path;
  inherited: Inherited;
}) {
  const set = (next: Json | undefined) => ctx.dispatch({ type: "set", path, value: next });
  const own = datasetClasses(ctx.root, schema);
  const classes = own ?? inherited.classes;
  const kind = fieldKind(ctx.root, schema);
  switch (kind) {
    case "nullable": {
      const inner = nullable(ctx.root, schema) ?? {};
      return (
        <Input
          ctx={ctx}
          schema={{ ...inner, ...(own ? { "x-accessmod-class": own } : {}) }}
          value={value ?? undefined}
          path={path}
          inherited={inherited}
        />
      );
    }
    case "string":
      if (classes !== null) {
        return (
          <DatasetSelect
            id={controlId(path)}
            ctx={ctx}
            classes={classes}
            value={value}
            onChange={set}
          />
        );
      }
      if (inherited.fields !== null) {
        return (
          <select
            id={controlId(path)}
            value={typeof value === "string" ? value : ""}
            onChange={(e) => set(e.target.value)}
          >
            <option value="">—</option>
            {inherited.fields.map((f) => (
              <option key={f} value={f}>
                {f}
              </option>
            ))}
          </select>
        );
      }
      return (
        <input
          id={controlId(path)}
          type="text"
          value={typeof value === "string" ? value : ""}
          onChange={(e) => set(e.target.value)}
        />
      );
    case "number":
    case "integer":
      return (
        <input
          id={controlId(path)}
          type="number"
          step={kind === "integer" ? 1 : "any"}
          value={typeof value === "number" ? value : ""}
          onChange={(e) => set(e.target.value === "" ? undefined : Number(e.target.value))}
        />
      );
    case "boolean":
      return (
        <input
          id={controlId(path)}
          type="checkbox"
          checked={value === true}
          onChange={(e) => set(e.target.checked)}
        />
      );
    case "enum": {
      const options = enumOptions(ctx.root, schema) ?? [];
      // Options by position: numbers stay numbers in the document.
      const current =
        typeof value === "string" || typeof value === "number" ? options.indexOf(value) : -1;
      return (
        <select
          id={controlId(path)}
          value={current}
          onChange={(e) => set(options[Number(e.target.value)])}
        >
          {current < 0 && <option value={-1}>—</option>}
          {options.map((o, i) => (
            <option key={String(o)} value={i}>
              {typeof o === "string" ? label(o) : o}
            </option>
          ))}
        </select>
      );
    }
    case "variants":
      return (
        <VariantInput
          ctx={ctx}
          list={variants(ctx.root, schema) ?? []}
          value={value}
          path={path}
          inherited={{ classes, fields: inherited.fields }}
        />
      );
    case "array":
      return <ArrayInput ctx={ctx} schema={schema} value={value} path={path} />;
    case "object":
      return (
        <div className="group">
          <ObjectFields
            ctx={ctx}
            schema={own ? { ...resolve(ctx.root, schema) } : schema}
            value={value}
            path={path}
            inherited={{ classes, fields: inherited.fields }}
          />
        </div>
      );
    case "any":
      return <code>{JSON.stringify(value)}</code>;
  }
}

function DatasetSelect({
  id,
  ctx,
  classes,
  value,
  onChange,
}: {
  id: string;
  ctx: Context;
  classes: string[];
  value: Json | undefined;
  onChange: (v: Json) => void;
}) {
  const choices = ctx.datasets.filter((d) => classes.includes(d.class));
  return (
    <select
      id={id}
      value={typeof value === "string" ? value : ""}
      onChange={(e) => onChange(e.target.value)}
    >
      <option value="">— {classes.join(" / ")} —</option>
      {choices.map((d) => (
        <option key={d.id} value={d.file}>
          {d.class} · {d.tags.join(" ")} (#{d.id})
        </option>
      ))}
    </select>
  );
}

function VariantInput({
  ctx,
  list,
  value,
  path,
  inherited,
}: {
  ctx: Context;
  list: Variant[];
  value: Json | undefined;
  path: Path;
  inherited: Inherited;
}) {
  const index = variantOf(list, value);
  const current = list[index];
  return (
    <div className="group">
      <select
        id={controlId(path)}
        value={index}
        onChange={(e) => {
          const next = list[Number(e.target.value)];
          if (next) ctx.dispatch({ type: "set", path, value: variantDefault(ctx.root, next) });
        }}
      >
        {index < 0 && <option value={-1}>—</option>}
        {list.map((v, i) => (
          <option key={`${v.kind}:${v.label}`} value={i}>
            {label(v.label)}
          </option>
        ))}
      </select>
      {current?.kind === "tagged" && (
        <ObjectFields
          ctx={ctx}
          schema={current.schema}
          value={value}
          path={path}
          inherited={inherited}
          skip={current.key}
        />
      )}
      {current?.kind === "external" &&
        properties(ctx.root, current.schema).map(([name, property]) => (
          <Input
            key={name}
            ctx={ctx}
            schema={property}
            value={getIn(value, [name])}
            path={[...path, name]}
            inherited={{ classes: inherited.classes, fields: inherited.fields }}
          />
        ))}
    </div>
  );
}

function ArrayInput({
  ctx,
  schema,
  value,
  path,
}: {
  ctx: Context;
  schema: Schema;
  value: Json | undefined;
  path: Path;
}) {
  const item = items(ctx.root, schema);
  const list = Array.isArray(value) ? value : [];
  return (
    <div className="array">
      {list.map((element, i) => (
        // Items have no identity of their own: the position is the key.
        // biome-ignore lint/suspicious/noArrayIndexKey: positional items
        <div className="array-item" key={i}>
          <Input ctx={ctx} schema={item} value={element} path={[...path, i]} inherited={NONE} />
          <button
            type="button"
            className="link small"
            onClick={() => ctx.dispatch({ type: "remove", path: [...path, i] })}
          >
            remove
          </button>
        </div>
      ))}
      <button
        type="button"
        className="link"
        onClick={() => ctx.dispatch({ type: "append", path, value: defaultValue(ctx.root, item) })}
      >
        + add
      </button>
    </div>
  );
}
