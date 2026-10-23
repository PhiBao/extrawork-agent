import { useEffect, useMemo, useState, type FormEvent } from "react";

type Span = { id: string; documentId: string; filename: string; quote: string };
type Finding = {
  key: string;
  objectionId?: string;
  kind: string;
  title: string;
  explanation: string;
  state: string;
  proposedDeltaCents: number;
  optionalDeltaCents: number;
  requiresDecision: boolean;
  decision?: string | null;
  reservation?: string | null;
  spans: Span[];
};
type Objection = { id: string; index: number; quote: string; topic: string; status: string; classifiedBy?: string };
type Line = {
  id: string;
  description: string;
  category: string;
  quantity: string;
  unit: string;
  amountCents: number;
  invoice?: string | null;
  duplicate: boolean;
  removed: boolean;
  adjustmentCents: number;
  adjustmentNote?: string | null;
  excerpts: Span[];
};
type Doc = { id: string; filename: string; kind: string; sha256: string; text?: string };
type Task = { id: string; title: string; status: string; findingKey: string };
type Proof = {
  joinMode?: string;
  joinDetail?: string;
  zetaris?: {
    state?: string;
    detail?: string;
    live?: boolean;
    requestId?: string;
    style?: string;
    totalCount?: number;
    extractsUsed?: number;
    queryClosed?: boolean;
    sql?: string;
  };
  meterless?: { state?: string; commit?: string; result?: { eventCount?: number; unchangedOnReplay?: boolean } };
  nemotron?: {
    state?: string;
    model?: string;
    detail?: string;
    rejected?: number;
    topics?: Array<{ quote: string; topic: string }>;
  };
  recordedAt?: string;
};
type CaseFile = {
  id: string;
  synthetic: boolean;
  revision: number;
  project: string;
  requestNumber: string;
  contractor: string;
  counterparty: string;
  returnedOn: string;
  summary: string;
  submittedCents: number;
  supportedCents: number;
  proposalCents: number;
  billingPosture: string;
  postureReason: string;
  workflow: string;
  objections: Objection[];
  findings: Finding[];
  lines: Line[];
  tasks: Task[];
  documents: Doc[];
  sourceAgreement: string;
  proof: Proof;
};

const CASES = [
  { id: "harbor-annex", request: "COR-184", project: "Harbor Annex" },
  { id: "northgate-mechanical", request: "CHG-207", project: "Northgate Plant" },
];

const money = (cents: number) =>
  new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(cents / 100);

function describeChange(before: CaseFile, next: CaseFile) {
  const parts: string[] = [];
  if (before.supportedCents !== next.supportedCents) {
    parts.push(`Supported ${money(before.supportedCents)} → ${money(next.supportedCents)}`);
  }
  const changed = before.objections
    .map((item, index) => [item, next.objections[index]] as const)
    .find(([oldItem, newItem]) => oldItem && newItem && oldItem.status !== newItem.status);
  if (changed) {
    parts.push(`Objection 0${changed[0].index}: ${changed[0].status} → ${changed[1].status}`);
  }
  if (!parts.length) {
    const labels: Record<string, string> = {
      accept: "correction accepted",
      reject: "correction rejected",
      request_approval: "approval request noted",
      apply_sheet_rate: "sheet rate applied",
    };
    const decided = before.findings
      .map((item) => [item, next.findings.find((candidate) => candidate.key === item.key)] as const)
      .find(([oldItem, newItem]) => newItem && oldItem.decision !== newItem.decision);
    if (decided?.[1]?.decision) {
      parts.push(`${decided[0].title} — ${labels[decided[1].decision] ?? decided[1].decision}`);
    }
  }
  if (!parts.length && before.findings.length !== next.findings.length) {
    parts.push("Findings recomputed");
  }
  return parts.join(" · ");
}

export function App() {
  const initial = new URLSearchParams(window.location.search).get("case") || "harbor-annex";
  const [caseId, setCaseId] = useState(initial);
  const [file, setFile] = useState<CaseFile | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("Reading the returned request");
  const [selected, setSelected] = useState("obj-1");
  const [source, setSource] = useState<Doc | null>(null);
  const [active, setActive] = useState<{ documentId: string; quote: string } | null>(null);
  const [change, setChange] = useState("");
  const [reloadToken, setReloadToken] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setBusy("Reading the returned request");
    setError("");
    fetch(`/api/cases/${caseId}`)
      .then((response) => {
        if (!response.ok) throw new Error("The case file could not be opened.");
        return response.json() as Promise<CaseFile>;
      })
      .then((next) => {
        if (cancelled) return;
        setFile(next);
        setBusy("");
        document.title = `${next.requestNumber} — ExtraWork`;
      })
      .catch((err: Error) => {
        if (cancelled) return;
        setError(err.message);
        setBusy("");
      });
    return () => {
      cancelled = true;
    };
  }, [caseId, reloadToken]);

  const objection = file?.objections.find((item) => item.id === selected) ?? file?.objections[0];
  const findings = useMemo(
    () => file?.findings.filter((item) => item.objectionId === objection?.id) ?? [],
    [file, objection],
  );

  const citations = useMemo(() => {
    const seen = new Set<string>();
    return findings
      .flatMap((item) => item.spans)
      .filter((span) => {
        const key = `${span.documentId}:${span.quote}`;
        if (seen.has(key)) return false;
        seen.add(key);
        return true;
      });
  }, [findings]);

  useEffect(() => {
    setActive(null);
  }, [objection?.id]);

  useEffect(() => {
    const span =
      (active && citations.find((item) => item.documentId === active.documentId && item.quote === active.quote)) ||
      citations[0];
    if (!span) {
      setSource(null);
      return;
    }
    fetch(`/api/cases/${caseId}/documents/${span.documentId}`)
      .then((response) => response.json())
      .then((doc: Doc) => setSource(doc))
      .catch(() => setSource(null));
  }, [citations, active, caseId]);

  const quote = active?.quote ?? citations[0]?.quote ?? "";

  function switchCase(id: string) {
    if (id === caseId) return;
    setCaseId(id);
    setSelected("obj-1");
    setActive(null);
    setSource(null);
    setChange("");
    window.history.replaceState(null, "", `?case=${id}`);
  }

  async function decide(findingKey: string, decision: string) {
    if (!file) return;
    setError("");
    const before = file;
    const response = await fetch(`/api/cases/${caseId}/decisions`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ findingKey, decision, expectedRevision: file.revision }),
    });
    if (response.status === 409) {
      setError("This file changed. The latest revision is showing.");
      setReloadToken((value) => value + 1);
      return;
    }
    if (!response.ok) {
      setError("That decision was not recorded.");
      return;
    }
    const next = (await response.json()) as CaseFile;
    setFile(next);
    setChange(describeChange(before, next));
  }

  async function upload(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!file) return;
    const form = event.currentTarget;
    const data = new FormData(form);
    const before = file;
    const response = await fetch(`/api/cases/${caseId}/sources`, { method: "POST", body: data });
    if (!response.ok) {
      setError(await response.text());
      return;
    }
    const next = (await response.json()) as CaseFile;
    setFile(next);
    setChange(describeChange(before, next));
    form.reset();
  }

  async function reset() {
    const response = await fetch(`/api/cases/${caseId}/reset`, { method: "POST" });
    if (!response.ok) {
      setError("The example could not be reset.");
      return;
    }
    setFile((await response.json()) as CaseFile);
    setChange("");
    setSelected("obj-1");
    setActive(null);
  }

  if (error && !file) return <main className="file"><p className="error">{error}</p></main>;
  if (!file) return <main className="file"><p>{busy}</p></main>;

  const primary = findings.find((item) => item.requiresDecision) ?? findings[0];
  const pending = file.findings.filter((item) => item.requiresDecision && !item.decision).length;
  const missing = file.findings.filter((item) => item.state === "missing").length;
  const zetaris = file.proof?.zetaris;
  const live = file.proof?.joinMode === "zetaris_live";
  const meterless = file.proof?.meterless;
  const steps = [
    ["01", "Read the return note", `${file.objections.length} objections`],
    ["02", live ? "Joined evidence in Zetaris" : "Joined local evidence", live ? `${zetaris?.extractsUsed ?? zetaris?.totalCount ?? 0} extracts` : "CSV + PostgreSQL"],
    ["03", "Checked the cost lines", `${file.lines.length} lines`],
    ["04", "Proposed a repair", pending ? `${pending} open` : "none open"],
    ["05", "Flagged missing records", `${missing} open`],
  ];
  const tasks = file.tasks.filter((task) => findings.some((item) => item.key === task.findingKey));

  return (
    <main className="file">
      <header className="filehead">
        <div className="topline">
          <div className="brand"><span className="mark" aria-hidden="true">E</span> ExtraWork</div>
          <nav className="files" aria-label="Project files">
            {CASES.map((item) => (
              <button
                key={item.id}
                aria-current={item.id === caseId}
                onClick={() => switchCase(item.id)}
              >
                {item.request}
                <span>{item.project}</span>
              </button>
            ))}
          </nav>
        </div>
        <p className="kicker">Returned {file.returnedOn}</p>
        <h1>{file.project}</h1>
        <p className="lede">{file.requestNumber} returned by {file.counterparty}. {file.contractor}. {file.summary}.</p>
      </header>
      <p className="banner">Synthetic project file. Nothing here was approved, billed, or recovered.</p>

      <ol className="run" aria-label="Agent run">
        {steps.map(([number, title, detail]) => (
          <li key={number}>
            <b>{number}</b>
            <strong>{title}</strong>
            <span>{detail}</span>
          </li>
        ))}
      </ol>
      {change ? <p className="change">Last change: {change}</p> : null}

      <div className="toolstrip">
        <span className={`tool-dot ${live ? "" : "off"}`} aria-hidden="true" />
        <strong>Zetaris</strong>
        <span>{zetaris?.state ?? "not run"}{zetaris?.style ? ` · ${zetaris.style}` : ""}{zetaris?.totalCount != null ? ` · ${zetaris.totalCount} rows` : ""}</span>
        <span>Meterless World Model · {meterless?.state ?? "not recorded"} · replay unchanged: {String(meterless?.result?.unchangedOnReplay ?? "not run")}</span>
        <span>Nemotron · {file.proof?.nemotron?.state ?? "not run"}{file.proof?.nemotron?.topics?.length ? ` · ${file.proof.nemotron.topics.length} topics` : ""}</span>
        {zetaris?.sql ? (
          <details>
            <summary>Query</summary>
            <code>{zetaris.sql}</code>
          </details>
        ) : null}
      </div>

      {error ? <p className="error">{error}</p> : null}
      <div className="workspace">
        <nav className="index" aria-label="Returned objections">
          <h2>Returned</h2>
          {file.objections.map((item) => (
            <button key={item.id} className="objection" aria-current={item.id === objection?.id} onClick={() => setSelected(item.id)}>
              <span className="num">0{item.index}</span>
              <span className="quote">{item.quote}</span>
              <span className={`status ${item.status}`}>{item.status}</span>
            </button>
          ))}
        </nav>
        <article className="repair">
          <h2>Repair</h2>
          <h3>{primary?.title ?? "No record answers this sentence yet."}</h3>
          <p className="explanation">{primary?.explanation}</p>
          {primary?.reservation ? <p className="reservation">{primary.reservation}</p> : null}
          {primary?.spans.length ? (
            <div className="cites primary-cites">
              {primary.spans.map((span) => (
                <button
                  key={span.id}
                  onClick={() => setActive({ documentId: span.documentId, quote: span.quote })}
                >
                  {span.filename}
                </button>
              ))}
            </div>
          ) : null}
          <div className="money">
            <p><span>Submitted</span><strong>{money(file.submittedCents)}</strong></p>
            <p><span>Supported now</span><strong>{money(file.supportedCents)}</strong></p>
            <p><span>{file.proposalCents !== file.supportedCents ? "Proposed total" : "Open corrections settled"}</span><strong>{money(file.proposalCents)}</strong></p>
          </div>
          <p>{file.postureReason}</p>
          <div className="actions">
            {findings.filter((item) => item.kind === "duplicate_charge" && !item.decision).map((item) => (
              <span key={item.key}>
                <button className="primary" onClick={() => decide(item.key, "accept")}>Remove the duplicate</button>
                <button onClick={() => decide(item.key, "reject")}>Keep both lines</button>
              </span>
            ))}
            {findings.filter((item) => item.kind === "quantity_shortfall" && !item.decision).map((item) => (
              <span key={item.key}>
                <button className="primary" onClick={() => decide(item.key, "accept")}>Remove the shortfall</button>
                <button onClick={() => decide(item.key, "reject")}>Keep the billed quantity</button>
              </span>
            ))}
            {findings.filter((item) => item.kind === "missing_rate_approval" && item.state === "missing" && item.decision !== "apply_sheet_rate").map((item) => (
              <span key={item.key}>
                <button onClick={() => decide(item.key, "request_approval")}>Request written approval</button>
                <button onClick={() => decide(item.key, "apply_sheet_rate")}>Use the sheet rate</button>
              </span>
            ))}
          </div>
          {tasks.length ? (
            <div className="task">
              <span className="status">Next request</span>
              {tasks.map((task) => (
                <p key={task.id}>{task.status === "requested" ? "Approval request noted." : task.title}</p>
              ))}
            </div>
          ) : null}
          <table>
            <caption className="index">Cost lines</caption>
            <thead><tr><th>Line</th><th>Description</th><th>Invoice</th><th className="right">Amount</th></tr></thead>
            <tbody>
              {file.lines.map((line) => (
                <tr key={line.id} className={line.removed ? "removed" : ""}>
                  <td>{line.id}{line.duplicate ? " · duplicate" : ""}</td>
                  <td>
                    {line.description}
                    {line.excerpts.map((span) => (
                      <span className="extract" key={span.id}>“{span.quote}” <em>{span.filename}</em></span>
                    ))}
                  </td>
                  <td>{line.invoice || "—"}</td>
                  <td className="right">
                    {money(line.amountCents)}
                    {line.adjustmentCents ? (
                      <span className="adjust">−{money(-line.adjustmentCents)} {line.adjustmentNote?.toLowerCase() ?? "unsupported"}</span>
                    ) : null}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {findings.filter((item) => item.key !== primary?.key).length ? (
            <section className="record">
              <h2>What else the records say</h2>
              <ul>
                {findings.filter((item) => item.key !== primary?.key).map((item) => (
                  <li key={item.key}>
                    <span className={`status ${item.state}`}>{item.state}</span>
                    <strong>{item.title}</strong>
                    <span>{item.explanation}</span>
                    {item.spans.length ? (
                      <span className="cites">
                        {item.spans.map((span) => (
                          <button
                            key={`${item.key}-${span.id}`}
                            onClick={() => setActive({ documentId: span.documentId, quote: span.quote })}
                          >
                            {span.filename}
                          </button>
                        ))}
                      </span>
                    ) : null}
                  </li>
                ))}
              </ul>
            </section>
          ) : null}
          {objection?.id === file.objections[0]?.id && file.findings.filter((item) => !item.objectionId).length ? (
            <section className="record unlinked">
              <h2>Also checked</h2>
              <ul>
                {file.findings
                  .filter((item) => !item.objectionId)
                  .map((item) => {
                    const task = file.tasks.find((entry) => entry.findingKey === item.key);
                    return (
                      <li key={item.key}>
                        <span className={`status ${item.state}`}>{item.state}</span>
                        <strong>{item.title}</strong>
                        <span>{item.explanation}</span>
                        {task ? (
                          <span className="taskline">
                            {task.status === "requested" ? "Approval request noted." : `Next: ${task.title}`}
                          </span>
                        ) : null}
                        {item.kind === "missing_rate_approval" && item.state === "missing" ? (
                          <span className="cites">
                            <button onClick={() => decide(item.key, "request_approval")}>Request approval</button>
                            <button onClick={() => decide(item.key, "apply_sheet_rate")}>Use the sheet rate</button>
                          </span>
                        ) : null}
                      </li>
                    );
                  })}
              </ul>
            </section>
          ) : null}
          <p className="status">{file.workflow.replaceAll("_", " ")} · {file.billingPosture.replaceAll("_", " ")}</p>
        </article>
        <aside className="source">
          <h2>Source</h2>
          <p>{source?.filename ?? "Select an objection to read its record."}</p>
          <pre>{highlight(source?.text ?? "", quote)}</pre>
        </aside>
      </div>
      <div className="tools">
        <a href={`/api/cases/${caseId}/export`}>Download the review packet</a>
        <a href={`/api/cases/${caseId}/export.zip`}>Download the record zip</a>
        <form className="upload" onSubmit={upload}>
          <label htmlFor="record">Add a record</label>
          <input id="record" name="file" type="file" accept=".csv,.txt,.eml,.md,.pdf" required />
          <button className="quiet" type="submit">Add</button>
        </form>
        <button className="quiet" onClick={reset}>Reset the example</button>
      </div>
      <details className="proof">
        <summary>How this was checked</summary>
        <p>
          {file.sourceAgreement}{" "}
          {live
            ? "Evidence extracts came from Zetaris through the Gateway."
            : "Evidence extracts were read from a separate CSV and joined to the project-line table."}
        </p>
        <p>World Model {meterless?.commit?.slice(0, 12) ?? "not recorded"} · {meterless?.state}. Replay unchanged: {String(meterless?.result?.unchangedOnReplay ?? "not run")}.</p>
        <p>
          Zetaris · {zetaris?.state ?? "not run"}
          {zetaris?.live ? ` · ${zetaris.style ?? "live"}` : ""}
          {zetaris?.totalCount != null ? ` · ${zetaris.totalCount} rows` : ""}
          {zetaris?.detail ? ` — ${zetaris.detail}` : ""}
        </p>
        <p>{file.proof?.nemotron?.detail ?? "Rules engine read the objections."}</p>
        {file.proof?.nemotron?.topics?.length ? (
          <ul className="topics">
            {file.proof.nemotron.topics.map((item) => (
              <li key={`${item.topic}-${item.quote}`}>
                <b>{item.topic}</b>
                <span>{item.quote}</span>
              </li>
            ))}
          </ul>
        ) : null}
      </details>
    </main>
  );
}

function highlight(text: string, quote: string) {
  if (!quote || !text.includes(quote)) return text;
  const [before, after] = text.split(quote);
  return <>{before}<mark>{quote}</mark>{after}</>;
}
