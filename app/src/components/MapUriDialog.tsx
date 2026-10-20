import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState, type FormEvent } from "react";

export interface UriMapping {
  from: string;
  to: string;
}

/** A modal for mapping a URI prefix such as `pathmap://PCM_MODELS/` to a local directory. */
export function MapUriDialog(props: {
  mappings: UriMapping[];
  onMap: (m: UriMapping) => Promise<boolean>;
  onClose: () => void;
}) {
  const { mappings, onMap, onClose } = props;
  const [from, setFrom] = useState("pathmap://");
  const [to, setTo] = useState("");
  const [busy, setBusy] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const d = dialog.current;
    if (d && !d.open) d.showModal();
  }, []);

  const pickDirectory = async () => {
    const dir = await openDialog({ directory: true, title: "Directory for the URI prefix" }).catch(() => null);
    if (typeof dir === "string") setTo(dir);
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!from.trim() || !to.trim()) return;
    setBusy(true);
    const ok = await onMap({ from: from.trim(), to: to.trim() });
    setBusy(false);
    if (ok) onClose();
  };

  return (
    <dialog ref={dialog} className="modal" onClose={onClose}>
      <form onSubmit={(e) => void submit(e)}>
        <h2>Map a URI prefix</h2>
        <p className="dim">
          Models and metamodels that refer to this prefix are read from the directory instead. References that could
          not be resolved yet are loaded right away.
        </p>
        <label className="field">
          <span>URI prefix</span>
          <input
            className="text-input mono"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
            placeholder="pathmap://PCM_MODELS/"
            spellCheck={false}
            autoFocus
          />
        </label>
        <label className="field">
          <span>Directory</span>
          <span className="field-row">
            <input
              className="text-input mono"
              value={to}
              onChange={(e) => setTo(e.target.value)}
              placeholder="/path/to/models/"
              spellCheck={false}
            />
            <button type="button" className="btn btn-small" onClick={() => void pickDirectory()}>
              Choose…
            </button>
          </span>
        </label>
        {mappings.length > 0 && (
          <div className="mapping-list">
            <span className="dim">Mapped in this session</span>
            <ul>
              {mappings.map((m) => (
                <li key={m.from}>
                  <code>{m.from}</code>
                  <span className="dim"> → </span>
                  <code>{m.to}</code>
                </li>
              ))}
            </ul>
          </div>
        )}
        <div className="modal-actions">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={busy || !from.trim() || !to.trim()}>
            Map prefix
          </button>
        </div>
      </form>
    </dialog>
  );
}
