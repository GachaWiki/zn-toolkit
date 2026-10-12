import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './App.css';

type TableMeta = Record<string, { rows: number; bytes: number }>;

const REGEN_SCRIPTS: { label: string; script: string; note: string }[] = [
  { label: 'regen_all_chars.py', script: 'regen_all_chars.py', note: 'all character pages data (skills/talents/stats)' },
  { label: 'gen_memories.py', script: 'gen_memories.py', note: '78 memory cards (slug map: memory_slug_gid_final.json)' },
  { label: 'gen_mem_tiers.py', script: 'wiki:gen_mem_tiers.py', note: 'memory tier list (repo root of gacha-wiki)' },
  { label: 'extract_rifts.py', script: 'extract_rifts.py', note: 'rift JSONs → output/rifts/' },
  { label: 'gen_rifts.py', script: 'gen_rifts.py', note: 'new rift pages (fill PATCHES map first)' },
  { label: 'gen_updates.py', script: 'gen_updates.py', note: 'patch-note entries (fill PATCHES map first)' },
];

const USED_TABLES = new Set([
  'character_data_info', 'character_skill_data_info', 'character_talent_info', 'passive_data_info',
  'buff_data_info', 'character_awaken_info', 'remembrance_info', 'remembrance_awaken_info',
  'remembrance_mod_info', 'remembrance_exp_info', 'translation_ds', 'translation_ui',
  'translation_uilabel', 'equipment_info', 'equipment_passive_info', 'rift_info', 'rift_setting_info',
  'gacha_info', 'gacha_ui_info', 'event_info', 'stack_item_info', 'global_variable_info',
  'equipment_level_value_info', 'pack_box_info', 'enemy_spawn_detail_info',
  'character_level_value_info', 'character_train_extra_ability_info',
]);

export default function App() {
  const [logs, setLogs] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [tables, setTables] = useState<TableMeta>({});
  const [filter, setFilter] = useState('');
  const [onlyUnused, setOnlyUnused] = useState(false);
  const [selected, setSelected] = useState('');
  const [rows, setRows] = useState<any[]>([]);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(0);
  const logRef = useRef<HTMLDivElement>(null);
  const PAGE = 25;

  useEffect(() => {
    const un = listen<string>('tool-log', (e) => {
      setLogs((prev) => [...prev.slice(-500), e.payload]);
    });
    return () => { un.then((f) => f()); };
  }, []);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [logs]);

  useEffect(() => {
    refreshTables();
  }, []);

  function refreshTables() {
    invoke<TableMeta>('table_counts').then(setTables).catch((e) => log('table scan failed: ' + e));
  }

  function log(line: string) {
    setLogs((prev) => [...prev.slice(-500), line]);
  }

  async function run(label: string, fn: () => Promise<string>) {
    if (busy) return;
    setBusy(true);
    log('=== ' + label + ' ===');
    try {
      const msg = await fn();
      log('✓ ' + msg);
    } catch (e: any) {
      log('✗ ERROR: ' + e);
    } finally {
      setBusy(false);
    }
  }

  const pull = () => run('Pull from BlueStacks', () => invoke<string>('pull_master'));
  const decrypt = () =>
    run('Decrypt + dump tables', () => invoke<string>('decrypt_dump').then((m) => { refreshTables(); return m; }));

  function openTable(name: string, p = 0) {
    setSelected(name);
    setPage(p);
    invoke<{ total: number; rows: any[] }>('read_table', { name, offset: p * PAGE, limit: PAGE })
      .then((r) => { setTotal(r.total); setRows(r.rows); })
      .catch((e) => { log('read failed: ' + e); setRows([]); });
  }

  const tableNames = Object.keys(tables)
    .filter((n) => n.toLowerCase().includes(filter.toLowerCase()))
    .filter((n) => (onlyUnused ? !USED_TABLES.has(n) : true))
    .sort();

  return (
    <div className="app">
      <header>
        <h1>Zone Nova Toolkit</h1>
        <span className="sub">master-data pipeline — pull → decrypt → dump → regenerate</span>
      </header>

      <section className="pipeline">
        <button disabled={busy} onClick={pull}>1 · Pull from BlueStacks</button>
        <button disabled={busy} onClick={decrypt}>2 · Decrypt + Dump tables</button>
        <button disabled={busy} onClick={refreshTables} className="ghost">Refresh table list</button>
        <span className="hint">blob magic: b5 61 2a 39 2a 41 b8 09 · BlueStacks must be running the game</span>
      </section>

      <section className="pipeline regen">
        <span className="label">Regenerate wiki data (python scripts):</span>
        {REGEN_SCRIPTS.map((s) => (
          <button
            key={s.script}
            disabled={busy}
            title={s.note}
            onClick={() => run(s.label, () => invoke('run_script', { script: s.script }))}
          >
            {s.label}
          </button>
        ))}
      </section>

      <div className="main">
        <div className="tables">
          <div className="table-controls">
            <input
              placeholder="filter tables…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
            <label>
              <input type="checkbox" checked={onlyUnused} onChange={(e) => setOnlyUnused(e.target.checked)} />
              not yet used on wiki
            </label>
          </div>
          <ul>
            {tableNames.map((n) => (
              <li
                key={n}
                className={n === selected ? 'sel' : ''}
                onClick={() => openTable(n, 0)}
              >
                <span className="tname">{n}</span>
                <span className="tmeta">
                  {!USED_TABLES.has(n) && <em>unused</em>} {tables[n].rows} rows
                </span>
              </li>
            ))}
          </ul>
        </div>

        <div className="viewer">
          {selected ? (
            <>
              <div className="viewer-head">
                <strong>{selected}</strong>
                <span>{total} rows · page {page + 1}/{Math.max(1, Math.ceil(total / PAGE))}</span>
                <span className="pager">
                  <button disabled={page === 0} onClick={() => openTable(selected, page - 1)}>‹ prev</button>
                  <button disabled={(page + 1) * PAGE >= total} onClick={() => openTable(selected, page + 1)}>next ›</button>
                </span>
              </div>
              <pre>{JSON.stringify(rows, null, 1)}</pre>
            </>
          ) : (
            <div className="empty">select a table to inspect</div>
          )}
        </div>
      </div>

      <div className="log" ref={logRef}>
        {logs.length === 0 ? <span className="dim">pipeline output appears here…</span> : logs.map((l, i) => <div key={i}>{l}</div>)}
      </div>
    </div>
  );
}
