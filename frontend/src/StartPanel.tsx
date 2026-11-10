import { useState } from "react";
import { Symbol, type Glyph } from "./Symbol";
import type { StartConfig } from "./lab";
const distributionHelp: Record<keyof StartConfig, string> = {
  hives: "Number of hives. Each hive has one queen.",
  drones_per_hive: "Number of dragonflies starting in each hive.",
  queen_energy: "Starting energy per queen, in terajoules (1 TJ = 10¹² joules).",
  queen_minerals: "Starting minerals per queen, in tonnes (1 tonne = 1,000 kg).",
  deposits: "Number of resource deposits. Each contains both energy and minerals.",
  energy_per_deposit: "Energy in each deposit, in terajoules (1 TJ = 10¹² joules).",
  minerals_per_deposit: "Minerals in each deposit, in kilograms.",
  seed: "World seed. The same seed and settings recreate the same layout.",
};
export function StartPanel({
  busy,
  onApply,
  onClose,
}: {
  busy: boolean;
  onApply: (config: StartConfig) => void;
  onClose: () => void;
}) {
  const [cfg, set] = useState<StartConfig>(() => ({
    seed: crypto.getRandomValues(new Uint32Array(1))[0],
    hives: 0,
    drones_per_hive: 3,
    queen_energy: 1e12,
    queen_minerals: 500,
    deposits: 0,
    energy_per_deposit: 250e9,
    minerals_per_deposit: 5000,
  }));
  const field = (
    key: keyof StartConfig,
    icon: Glyph,
    label: string,
    max: number,
    scale = 1,
  ) => (
    <label data-help={distributionHelp[key]}>
      <Symbol name={icon} size={18} />
      <input
        type="number"
        min="0"
        max={max}
        step={scale === 1 ? 1 : "any"}
        value={cfg[key] / scale}
        aria-label={label}
        disabled={busy}
        onChange={(e) => set({ ...cfg, [key]: Number(e.target.value) * scale })}
      />
      {scale !== 1 && (
        <small>
          ×10<sup>{Math.log10(scale)}</sup>
        </small>
      )}
    </label>
  );
  const valid =
    Object.values(cfg).every((n) => Number.isFinite(n) && n >= 0) &&
    Number.isInteger(cfg.hives) &&
    Number.isInteger(cfg.drones_per_hive) &&
    Number.isInteger(cfg.deposits) &&
    Number.isSafeInteger(cfg.seed) &&
    cfg.hives * (cfg.drones_per_hive + 1) <= 100 &&
    cfg.hives <= 100 &&
    cfg.drones_per_hive <= 99 &&
    cfg.deposits <= 1000 &&
    cfg.queen_energy <= 3e12 &&
    cfg.queen_minerals <= 1e6 &&
    cfg.energy_per_deposit <= 1e16 &&
    cfg.minerals_per_deposit <= 1e9;
  return (
    <section className="start-panel" aria-label="Starting world configuration">
      <div>
        {field("hives", "queen", "Starting hive count", 100)}
        {field(
          "drones_per_hive",
          "dragonfly",
          "Starting dragonflies per hive",
          99,
        )}
      </div>
      <div>
        <span data-help="Starting stores in each queen." className="start-context"><Symbol name="queen" /></span>
        {field(
          "queen_energy",
          "energy",
          "Starting queen energy in terajoules",
          3,
          1e12,
        )}
        {field(
          "queen_minerals",
          "mineral",
          "Starting queen minerals in tonnes",
          1000,
          1000,
        )}
      </div>
      <div>
        {field("deposits", "collect", "Starting mixed deposit count", 1000)}
      </div>
      <div>
        <span data-help="Contents of each resource deposit." className="start-context"><Symbol name="collect" /></span>
        {field(
          "energy_per_deposit",
          "energy",
          "Energy per mixed deposit in terajoules",
          10000,
          1e12,
        )}
        {field(
          "minerals_per_deposit",
          "mineral",
          "Minerals per mixed deposit in kilograms",
          1e9,
        )}
      </div>
      <div>
        {field("seed", "mark", "Starting world seed", 4294967295)}
        <button
          className="icon-button"
          aria-label="Generate configured world"
          data-help="Generate a new world using these settings."
          disabled={busy || !valid}
          onClick={() => onApply(cfg)}
        >
          <Symbol name="reset" />
        </button>
        <button
          className="icon-button"
          aria-label="Close starting world configuration"
          data-help="Close without generating a new world."
          disabled={busy}
          onClick={onClose}
        >
          <Symbol name="close" />
        </button>
      </div>
    </section>
  );
}
