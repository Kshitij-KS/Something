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

  const reload = () => {
    void invoke<ReviewItem[]>("list_review")
      .then(setItems)
      .catch(() => setItems([]));
  };

  useEffect(() => {
    reload();
  }, []);

  return (
    <section>
      <h1>Review</h1>
      <p>
        Low-confidence clauses stay here. They are never surfaced as
        notifications.
      </p>
      {items.length === 0 ? <p className="empty">Nothing to review.</p> : null}
      <ul className="queue">
        {items.map((item) => (
          <li key={item.id}>
            <p>{item.text}</p>
            <p className="meta">
              {item.source_app}
              {item.recipient ? ` · ${item.recipient}` : ""} · score{" "}
              {item.score}
            </p>
            <div className="row">
              <button
                type="button"
                onClick={() => {
                  void invoke("review_promise", {
                    id: item.id,
                    action: "promote",
                    text: item.text,
                  }).then(reload);
                }}
              >
                Promote
              </button>
              <button
                type="button"
                onClick={() => {
                  void invoke("review_promise", {
                    id: item.id,
                    action: "reject",
                    text: item.text,
                  }).then(reload);
                }}
              >
                Not a promise
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
