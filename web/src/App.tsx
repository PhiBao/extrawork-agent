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
type Objection = { id: string; index: number; quote: string; topic: string; status: string };
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
  excerpts: Span[];
};
type Doc = { id: string; filename: string; kind: string; sha256: string; text?: string };
type Proof = {
  joinMode?: string;
  joinDetail?: string;
  zetaris?: { state?: string; detail?: string; live?: boolean; requestId?: string };
  meterless?: { state?: string; commit?: string; result?: { eventCount?: number; unchangedOnReplay?: boolean } };
  nemotron?: { state?: string; model?: string; detail?: string };
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
  documents: Doc[];
  sourceAgreement: string;
  proof: Proof;
};

const money = (cents: number) =>
  new Intl.NumberFormat("en-US", { style: "currency", currency: "USD" }).format(cents / 100);

export function App() {
  const [file, setFile] = useState<CaseFile | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("Reading the returned request");
  const [selected, setSelected] = useState("obj-1");
  const [source, setSource] = useState<Doc | null>(null);
  const [quote, setQuote] = useState("");

  async function load() {
    const response = await fetch("/api/cases/harbor-annex");
    if (!response.ok) throw new Error("The case file could not be opened.");
    const next = (await response.json()) as CaseFile;
    setFile(next);
    setBusy("");
    return next;
  }

  useEffect(() => {
    load().catch((err: Error) => setError(err.message));
  }, []);

  const objection = file?.objections.find((item) => item.id === selected) ?? file?.objections[0];
  const findings = useMemo(
    () => file?.findings.filter((item) => item.objectionId === objection?.id) ?? [],
    [file, objection],
  );

  useEffect(() => {
    const span = findings.flatMap((item) => item.spans)[0];
    if (!span) return;
    setQuote(span.quote);
    fetch(`/api/cases/harbor-annex/documents/${span.documentId}`)
      .then((response) => response.json())
      .then((doc: Doc) => setSource(doc))
      .catch(() => setSource(null));
  }, [findings]);

  async function decide(findingKey: string, decision: string) {
    if (!file) return;
    setError("");
    const response = await fetch("/api/cases/harbor-annex/decisions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ findingKey, decision, expectedRevision: file.revision }),
    });
    if (response.status === 409) {
      setError("This file changed. The latest revision is showing.");
      await load();
      return;
    }
    if (!response.ok) {
      setError("That decision was not recorded.");
      return;
    }
    setFile((await response.json()) as CaseFile);
  }

  async function upload(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    const response = await fetch("/api/cases/harbor-annex/sources", { method: "POST", body: data });
    if (!response.ok) {
      setError(await response.text());
      return;
    }
    setFile((await response.json()) as CaseFile);
    form.reset();
  }

  if (error && !file) return <main className="file"><p className="error">{error}</p></main>;
  if (!file) return <main className="file"><p>{busy}</p></main>;

  const primary = findings.find((item) => item.requiresDecision) ?? findings[0];

  return (
    <main className="file">
      <header className="filehead">
        <div className="brand"><span className="mark" aria-hidden="true">E</span> ExtraWork</div>
        <p className="kicker">Returned {file.returnedOn}</p>
        <h1>{file.project}</h1>
        <p className="lede">{file.requestNumber} returned by {file.counterparty}. {file.contractor}. {file.summary}.</p>
      </header>
      <p className="banner">Synthetic project file. Nothing here was approved, billed, or recovered.</p>
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
            {findings.filter((item) => item.kind === "missing_rate_approval" && item.state === "missing" && item.decision !== "apply_sheet_rate").map((item) => (
              <span key={item.key}>
                <button onClick={() => decide(item.key, "request_approval")}>Request written approval</button>
                <button onClick={() => decide(item.key, "apply_sheet_rate")}>Use the $125 sheet rate</button>
              </span>
            ))}
          </div>
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
                  <td className="right">{money(line.amountCents)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="status">{file.workflow.replaceAll("_", " ")} · {file.billingPosture.replaceAll("_", " ")}</p>
        </article>
        <aside className="source">
          <h2>Source</h2>
          <p>{source?.filename ?? "Select an objection to read its record."}</p>
          <pre>{highlight(source?.text ?? "", quote)}</pre>
        </aside>
      </div>
      <div className="tools">
        <a href="/api/cases/harbor-annex/export">Download the review packet</a>
        <a href="/api/cases/harbor-annex/export.zip">Download the record zip</a>
        <form className="upload" onSubmit={upload}>
          <label className="quiet" htmlFor="record">Add a record</label>
          <input id="record" name="file" type="file" accept=".csv,.txt,.eml,.md,.pdf" required />
          <button className="quiet" type="submit">Add</button>
        </form>
        <button className="quiet" onClick={() => fetch("/api/cases/harbor-annex/reset", { method: "POST" }).then((r) => r.json()).then(setFile)}>Reset the example</button>
      </div>
      <details className="proof">
        <summary>How this was checked</summary>
        <p>{file.sourceAgreement} Evidence extracts were read from a separate CSV and joined to the project-line table. {file.proof?.zetaris?.detail ?? file.proof?.zetaris?.state}</p>
        <p>World Model {file.proof?.meterless?.commit ?? "not recorded"} · {file.proof?.meterless?.state}. Replay unchanged: {String(file.proof?.meterless?.result?.unchangedOnReplay ?? "not run")}.</p>
        <p>{file.proof?.nemotron?.detail ?? "Rules engine read the objections."}</p>
      </details>
    </main>
  );
}

function highlight(text: string, quote: string) {
  if (!quote || !text.includes(quote)) return text;
  const [before, after] = text.split(quote);
  return <>{before}<mark>{quote}</mark>{after}</>;
}
