import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api/client";
import { useProjects } from "../api/queries";
import { t } from "../app/i18n";
import { navigate } from "../app/route";
import type { Api } from "../generated/types";
import { Issues, Loading, when } from "./common";

export function ProjectsScreen() {
  const projects = useProjects();
  return (
    <section>
      <h1>{t("section.projects")}</h1>
      {projects.isPending && <Loading />}
      <Issues error={projects.error} />
      {projects.data?.length === 0 && <p className="muted">{t("project.none")}</p>}
      <div className="cards">
        {projects.data?.map((p) => (
          <button
            type="button"
            className="card"
            key={p.slug}
            onClick={() => navigate({ project: p.slug, section: "data", item: null })}
          >
            <strong>{p.info.name}</strong>
            <span className="muted">{p.info.description}</span>
            <span className="muted small">
              {p.info.grid.cols} × {p.info.grid.rows} cells · {when(p.info.created)}
            </span>
          </button>
        ))}
      </div>
      <NewProject />
    </section>
  );
}

function NewProject() {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [dem, setDem] = useState("");
  const create = useMutation({
    mutationFn: () => api.command<Api.CreatedProject>("project.create", { name, description, dem }),
    onSuccess: (created) => navigate({ project: created.slug, section: "data", item: null }),
  });
  return (
    <form
      className="panel"
      onSubmit={(e) => {
        e.preventDefault();
        create.mutate();
      }}
    >
      <h2>{t("project.new")}</h2>
      <label>
        {t("project.name")}
        <input value={name} onChange={(e) => setName(e.target.value)} required />
      </label>
      <label>
        {t("project.description")}
        <input value={description} onChange={(e) => setDescription(e.target.value)} />
      </label>
      <label>
        {t("project.dem")}
        <input
          value={dem}
          onChange={(e) => setDem(e.target.value)}
          placeholder="/path/to/dem.tif"
          required
        />
      </label>
      <button type="submit" disabled={create.isPending}>
        {t("project.create")}
      </button>
      <Issues error={create.error} />
    </form>
  );
}
