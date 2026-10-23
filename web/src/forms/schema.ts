// What a form needs from a JSON Schema (as generated from the Rust types):
// field kinds, variants of enums, defaults, dataset classes, and immutable
// edits of the document. Pure functions: the form renders what they say.

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type Schema = { readonly [key: string]: unknown };
export type Path = readonly (string | number)[];

/** Key of the classes a dataset field accepts (Rust `class::SCHEMA_KEY`). */
export const CLASS_KEY = "x-accessmod-class";

const isObject = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);
const schemas = (v: unknown): Schema[] => (Array.isArray(v) ? v.filter(isObject) : []);

/** `schema` with its `$ref` followed (repeatedly). */
export function resolve(root: Schema, schema: Schema): Schema {
  let current = schema;
  for (let depth = 0; depth < 32; depth += 1) {
    const ref = current.$ref;
    if (typeof ref !== "string" || !ref.startsWith("#/")) return current;
    let target: unknown = root;
    for (const part of ref.slice(2).split("/")) {
      target = isObject(target) ? target[part] : undefined;
    }
    if (!isObject(target)) return current;
    // Keep the referring schema's annotations (description, classes).
    const { $ref: _, ...annotations } = current;
    current = { ...target, ...annotations };
  }
  return current;
}

const isNull = (s: Schema) => s.type === "null";

/** The schema of an optional value (`anyOf: [T, null]`, `type: [T, "null"]`). */
export function nullable(root: Schema, schema: Schema): Schema | null {
  const s = resolve(root, schema);
  for (const key of ["anyOf", "oneOf"]) {
    const alternatives = schemas(s[key]);
    if (alternatives.length === 2 && alternatives.some(isNull)) {
      const inner = alternatives.find((a) => !isNull(a));
      if (inner) {
        const { [key]: _, ...annotations } = s;
        return resolve(root, { ...inner, ...annotations });
      }
    }
  }
  if (Array.isArray(s.type) && s.type.includes("null")) {
    const types = s.type.filter((t) => t !== "null");
    return { ...s, type: types.length === 1 ? types[0] : types };
  }
  return null;
}

/** A choice of an enum. */
export type Choice = string | number;

/** Choices of an enum: plain `enum` (strings or numbers), or `oneOf` of
 * string constants. */
export function enumOptions(root: Schema, schema: Schema): Choice[] | null {
  const s = resolve(root, schema);
  if (
    Array.isArray(s.enum) &&
    s.enum.length > 0 &&
    s.enum.every((v) => typeof v === "string" || typeof v === "number")
  ) {
    return s.enum;
  }
  const alternatives = schemas(s.oneOf);
  if (alternatives.length === 0) return null;
  const values = alternatives.map(constant);
  return values.every((v): v is string => typeof v === "string") ? values : null;
}

function constant(s: Schema): string | null {
  if (typeof s.const === "string") return s.const;
  if (Array.isArray(s.enum) && s.enum.length === 1 && typeof s.enum[0] === "string")
    return s.enum[0];
  return null;
}

/** An alternative of an enum with data. */
export type Variant =
  | { kind: "constant"; label: string; value: string }
  | { kind: "tagged"; label: string; key: string; value: string; schema: Schema }
  | { kind: "external"; label: string; tag: string; schema: Schema };

/** Alternatives of an enum with data (serde tagged, external or unit). */
export function variants(root: Schema, schema: Schema): Variant[] | null {
  const s = resolve(root, schema);
  const alternatives = schemas(s.oneOf ?? s.anyOf);
  if (alternatives.length === 0 || nullable(root, s) !== null || enumOptions(root, s) !== null) {
    return null;
  }
  const out: Variant[] = [];
  for (const raw of alternatives) {
    const alt = resolve(root, raw);
    const value = constant(alt);
    if (value !== null) {
      out.push({ kind: "constant", label: value, value });
      continue;
    }
    for (const option of enumOptions(root, alt) ?? []) {
      if (typeof option === "string") out.push({ kind: "constant", label: option, value: option });
    }
    if (enumOptions(root, alt) !== null) continue;
    const properties = isObject(alt.properties) ? alt.properties : {};
    const tag = Object.entries(properties).find(([, p]) => isObject(p) && constant(p) !== null);
    if (tag) {
      const [key, p] = tag;
      const v = constant(p as Schema) ?? "";
      out.push({ kind: "tagged", label: v, key, value: v, schema: alt });
      continue;
    }
    const required = Array.isArray(alt.required) ? alt.required : [];
    const names = Object.keys(properties);
    if (names.length === 1 && required.length === 1 && typeof names[0] === "string") {
      const name = names[0];
      out.push({ kind: "external", label: name, tag: name, schema: alt });
      continue;
    }
    return null;
  }
  return out;
}

/** Index of the variant `value` is, or -1. */
export function variantOf(variantList: Variant[], value: Json | undefined): number {
  return variantList.findIndex((v) => {
    if (v.kind === "constant") return value === v.value;
    if (v.kind === "tagged") return isObject(value) && value[v.key] === v.value;
    return isObject(value) && v.tag in value;
  });
}

export type FieldKind =
  | "string"
  | "number"
  | "integer"
  | "boolean"
  | "enum"
  | "array"
  | "object"
  | "variants"
  | "nullable"
  | "any";

export function fieldKind(root: Schema, schema: Schema): FieldKind {
  const s = resolve(root, schema);
  if (nullable(root, s) !== null) return "nullable";
  if (enumOptions(root, s) !== null) return "enum";
  if (variants(root, s) !== null) return "variants";
  switch (s.type) {
    case "string":
      return "string";
    case "number":
      return "number";
    case "integer":
      return "integer";
    case "boolean":
      return "boolean";
    case "array":
      return "array";
    case "object":
      return "object";
    default:
      return isObject(s.properties) ? "object" : "any";
  }
}

/** Properties of an object schema, in order, with whether each is required. */
export function properties(root: Schema, schema: Schema): [string, Schema, boolean][] {
  const s = resolve(root, schema);
  const props = isObject(s.properties) ? s.properties : {};
  const required = new Set(Array.isArray(s.required) ? s.required : []);
  return Object.entries(props)
    .filter((entry): entry is [string, Schema] => isObject(entry[1]))
    .map(([name, p]) => [name, p, required.has(name)]);
}

/** Item schema of an array schema. */
export function items(root: Schema, schema: Schema): Schema {
  const s = resolve(root, schema);
  return isObject(s.items) ? s.items : {};
}

/** Data classes a dataset field accepts, if it is one. */
export function datasetClasses(root: Schema, schema: Schema): string[] | null {
  const s = resolve(root, schema);
  const classes = s[CLASS_KEY];
  return Array.isArray(classes) && classes.every((c) => typeof c === "string") ? classes : null;
}

/** A value for a new field of `schema`: its default, or the first choice. */
export function defaultValue(root: Schema, schema: Schema): Json {
  const s = resolve(root, schema);
  if ("default" in s) return s.default as Json;
  switch (fieldKind(root, s)) {
    case "nullable":
      return null;
    case "enum":
      return enumOptions(root, s)?.[0] ?? "";
    case "variants": {
      const first = variants(root, s)?.[0];
      return first ? variantDefault(root, first) : null;
    }
    case "string":
      return "";
    case "number":
    case "integer":
      return 0;
    case "boolean":
      return false;
    case "array":
      return [];
    case "object": {
      const out: { [key: string]: Json } = {};
      for (const [name, p, required] of properties(root, s)) {
        if (required) out[name] = defaultValue(root, p);
      }
      return out;
    }
    case "any":
      return null;
  }
}

/** A new value of variant `v`. */
export function variantDefault(root: Schema, v: Variant): Json {
  switch (v.kind) {
    case "constant":
      return v.value;
    case "tagged": {
      const value = defaultValue(root, v.schema);
      return isObject(value) ? { ...value, [v.key]: v.value } : { [v.key]: v.value };
    }
    case "external": {
      const inner = properties(root, v.schema)[0];
      return { [v.tag]: inner ? defaultValue(root, inner[1]) : null };
    }
  }
}

// ---- immutable edits --------------------------------------------------------

export function getIn(value: Json | undefined, path: Path): Json | undefined {
  let current = value;
  for (const key of path) {
    if (typeof key === "number") current = Array.isArray(current) ? current[key] : undefined;
    else current = isObject(current) ? (current as { [k: string]: Json })[key] : undefined;
  }
  return current;
}

/** `value` with `path` set to `next`; `undefined` removes an object key. */
export function setIn(value: Json | undefined, path: Path, next: Json | undefined): Json {
  const [key, ...rest] = path;
  if (key === undefined) return next ?? null;
  if (typeof key === "number") {
    const array = Array.isArray(value) ? [...value] : [];
    array[key] = setIn(array[key], rest, next);
    return array;
  }
  const object: { [k: string]: Json } = isObject(value)
    ? { ...(value as { [k: string]: Json }) }
    : {};
  const child = rest.length === 0 ? next : setIn(object[key], rest, next);
  if (child === undefined) delete object[key];
  else object[key] = child;
  return object;
}

/** `value` without the array item or object key at `path`. */
export function removeIn(value: Json, path: Path): Json {
  const parent = path.slice(0, -1);
  const last = path.at(-1);
  const container = getIn(value, parent);
  if (typeof last === "number" && Array.isArray(container)) {
    return setIn(
      value,
      parent,
      container.filter((_, i) => i !== last),
    );
  }
  if (typeof last === "string") return setIn(value, path, undefined);
  return value;
}
