import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type ReviewItem = {
  id: number;
  text: string;
  source_app: string;
  recipient?: string | null;
  score: number;
  status: string;
};

export function ReviewQueue() {
  const [items, setItems] = useState<ReviewItem[]>([]);
  const [drafts, setDrafts] = useState<Record<number, string>>({});
  const [message, setMessage] = useState<string | null>(null);

  const reload = () => {
    void invoke<ReviewItem[]>("list_review")
      .then((next) => {
        setItems(next);
        setDrafts(Object.fromEntries(next.map((item) => [item.id, item.text])));
      })
      .catch(() => setItems([]));
  };

  useEffect(() => {
    reload();
  }, []);

  const review = (item: ReviewItem, action: "promote" | "reject" | "edit") => {
    const text = drafts[item.id] ?? item.text;
    setMessage(null);
    void invoke("review_promise", {
      id: item.id,
      action,
      text,
    })
      .then(() => reload())
      .catch((error: unknown) => setMessage(String(error)));
  };

  return (
    <section>
      <h1>Review</h1>
      <p>
        Low-confidence clauses stay here. They receive no triggers and are never
        surfaced until you promote them.
      </p>
      {items.length === 0 ? <p className="empty">Nothing to review.</p> : null}
      <ul className="queue">
        {items.map((item) => (
          <li key={item.id}>
            <textarea
              aria-label={`Promise ${item.id}`}
              value={drafts[item.id] ?? item.text}
              onChange={(event) =>
                setDrafts((current) => ({
                  ...current,
                  [item.id]: event.target.value,
                }))
              }
            />
            <p className="meta">
              {item.source_app}
              {item.recipient ? ` · ${item.recipient}` : ""} · score{" "}
              {item.score}
            </p>
            <div className="row">
              <button type="button" onClick={() => review(item, "promote")}>
                Promote
              </button>
              <button type="button" onClick={() => review(item, "edit")}>
                Save &amp; promote
              </button>
              <button type="button" onClick={() => review(item, "reject")}>
                Not a promise
              </button>
            </div>
          </li>
        ))}
      </ul>
      {message ? <p>{message}</p> : null}
    </section>
  );
}
