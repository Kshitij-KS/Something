import { invoke } from "@tauri-apps/api/core";

type Props = { onDone: () => void };

export function Onboarding({ onDone }: Props) {
  return (
    <main className="onboarding">
      <p className="eyebrow">Local-only setup</p>
      <h1>Install the capture extension</h1>
      <ol>
        <li>Open Chrome → Extensions → Load unpacked</li>
        <li>Select the Callback `extension/dist` folder</li>
        <li>Keep this app running so the native host can register</li>
      </ol>
      <p>
        Autostart is off until you enable it in Settings. Callback stays silent
        for 30 minutes after this screen so it can collect before it speaks.
      </p>
      <p>
        Unsigned Windows builds may show SmartScreen. Package managers do not
        remove that warning.
      </p>
      <button
        type="button"
        onClick={() => {
          void invoke("complete_onboarding").then(onDone);
        }}
      >
        Extension is installed
      </button>
    </main>
  );
}
