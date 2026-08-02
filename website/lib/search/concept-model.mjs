// @ts-check

export const MODEL_ID = 'pmoke-domain-v1';

/** @type {[string, string[]][]} */
const CONCEPTS = [
  ['overview', ['overview', 'what is pmoke', 'design priorities', 'reproducibility', '概要', 'pmoke とは', '設計方針', '再現可能']],
  ['quickstart', ['quickstart', 'quick start', 'first run', 'complete analysis run', 'クイックスタート', '最初の解析', '一括実行']],
  ['install', ['install', 'installation', 'setup', 'build from source', 'prerequisite', 'analysis-only installation', '導入', 'インストール', 'セットアップ', 'ビルド', '前提環境', '解析専用 build']],
  ['feature', ['feature flag', 'cargo feature', 'hardware feature', 'feature selection', 'visa compile', '機能フラグ', 'feature flag', 'cargo feature', 'feature の選択', 'visa compile']],
  ['config', ['config', 'configuration', 'toml', 'setting', '設定', '構成', 'toml']],
  ['validate', ['validate', 'validation', 'diagnostic', 'invalid', 'migrate legacy', 'field path', '検証', '診断', '設定ミス', '不正', 'legacy 設定の移行', 'field path']],
  ['schema', ['schema', 'json schema', 'field reference', 'parameter reference', 'スキーマ', 'schema', 'フィールド', 'パラメーター']],
  ['connect', ['connect', 'connection', 'transport', '通信', '接続', 'トランスポート']],
  ['tcpip', ['tcpip', 'tcp/ip', 'ethernet', 'network instrument', 'socket', 'lan', 'ネットワーク', 'tcp', 'lan 接続']],
  ['gpib', ['gpib', 'usb-gpib', 'ieee-488', 'pad', 'gpib', 'usb gpib', 'ieee 488']],
  ['prologix', ['prologix', 'ethernet-gpib', 'usb prologix', 'プロロジックス', 'prologix']],
  ['serial', ['serial', 'uart', 'baud', 'tty', 'com port', 'シリアル', 'ボーレート', 'tty', 'com ポート']],
  ['scope', ['oscilloscope', 'scope', 'dho5108', 'waveform source', 'オシロスコープ', 'オシロ', 'dho5108']],
  ['generator', ['function generator', 'generator', 'wf1946b', 'signal source', 'ファンクションジェネレーター', '信号発生器', 'wf1946b']],
  ['multimeter', ['multimeter', 'keithley', '2010', 'dmm', 'マルチメーター', 'デジタルマルチメーター', 'keithley']],
  ['acquire', ['acquire', 'acquisition', 'fetch data', 'capture', 'record waveform', '測定', '取得', '収録', '波形取得']],
  ['waveform', ['waveform', 'sample rate', 'timebase', 'csv', 'signal data', '波形', 'サンプルレート', '時間軸', 'csv']],
  ['reference', ['reference signal', 'reference fft', 'f_ref', 'frequency reference', '参照信号', '基準信号', '参照 fft', '基準周波数']],
  ['sensor', ['sensor', 'field channel', 'current channel', 'センサー', '磁場チャンネル', '電流チャンネル']],
  ['lockin', ['lock-in', 'lockin', 'demodulation', 'in-phase', 'quadrature', '同期検波', 'ロックイン', '復調', '同相', '直交']],
  ['filter', ['filter', 'low-pass', 'cutoff', 'enbw', 'time constant', 'フィルター', 'ローパス', '遮断周波数', '時定数']],
  ['boxcar', ['boxcar', 'moving average', 'window cycle', 'half window', '移動平均', 'ボックスカー', '窓幅', 'window cycle']],
  ['iir', ['iir', 'sync_iir_zero_phase', 'zero phase', 'butterworth', '同期 iir', 'ゼロ位相', 'sync iir']],
  ['phase', ['phase', 'rotation', 'omega_t0', '位相', '位相回転', '回転角']],
  ['kerr', ['kerr', 'magneto-optic', 'moke', 'rotation angle', 'カー角', '磁気光学', 'moke']],
  ['harmonics', ['harmonic', 'harmonics', 'six harmonic', '高調波', 'ハーモニクス', '六次高調波']],
  ['analysis', ['analyze', 'analysis pipeline', 'processing', '解析', '解析パイプライン', '処理']],
  ['plot', ['plot', 'graph', 'matplotlib', 'visualize', 'プロット', 'グラフ', '可視化']],
  ['tui', ['tui', 'monitor', 'dashboard', 'terminal ui', 'モニター', 'ダッシュボード', 'ターミナル ui']],
  ['cli', ['cli', 'command', 'subcommand', 'option', 'コマンド', 'サブコマンド', 'オプション']],
  ['instrument', ['instrument registry', 'instruments list', 'instrument query', 'device registry', '装置登録', '測定装置', 'instrument query']],
  ['benchmark', ['benchmark', 'latency', 'p50', 'p90', 'p99', 'transport bench', 'ベンチマーク', '遅延', 'レイテンシ']],
  ['troubleshoot', ['troubleshoot', 'doctor', 'error', 'timeout', 'failed', 'トラブルシュート', 'doctor', 'エラー', 'タイムアウト', '失敗']],
  ['wasm', ['wasm', 'webassembly', 'browser', 'browser tool', 'web tool', 'simulator', 'native parity', 'ブラウザ', 'ブラウザツール', 'webassembly', 'wasm', '数値一致']],
  ['ai', ['ai agent', 'llms.txt', 'llm', 'machine resource', 'copilot', 'ai エージェント', 'llms.txt', '機械向け']],
  ['privacy', ['privacy', 'telemetry', 'query upload', 'local search', 'プライバシー', 'テレメトリ', '外部送信', 'ローカル検索']],
];

const HASH_DIMENSIONS = 48;
const CONCEPT_WEIGHT = 3;
const HASH_WEIGHT = 0.18;

export const VECTOR_DIMENSIONS = CONCEPTS.length + HASH_DIMENSIONS;

/** @param {string} value */
export function normalizeSearchText(value) {
  return value
    .normalize('NFKC')
    .toLocaleLowerCase('en-US')
    .replace(/[_/]+/gu, ' ')
    .replace(/[^\p{Letter}\p{Number}.+-]+/gu, ' ')
    .replace(/\s+/gu, ' ')
    .trim();
}

/** @param {string} value */
export function embedSearchText(value) {
  const normalized = normalizeSearchText(value);
  const vector = new Array(VECTOR_DIMENSIONS).fill(0);

  for (let index = 0; index < CONCEPTS.length; index += 1) {
    const aliases = CONCEPTS[index][1];
    let matches = 0;
    for (const alias of aliases) matches += countOccurrences(normalized, normalizeSearchText(alias));
    if (matches > 0) vector[index] = CONCEPT_WEIGHT * (1 + Math.log1p(matches));
  }

  for (const feature of lexicalFeatures(normalized)) {
    const bucket = CONCEPTS.length + (fnv1a(feature) % HASH_DIMENSIONS);
    vector[bucket] += HASH_WEIGHT;
  }

  const norm = Math.hypot(...vector);
  if (norm === 0) return vector;
  return vector.map((value) => Math.round((value / norm) * 1_000_000) / 1_000_000);
}

/** @param {string} value */
export function matchedConcepts(value) {
  const normalized = normalizeSearchText(value);
  return CONCEPTS.flatMap(([id, aliases]) =>
    aliases.some((alias) => normalized.includes(normalizeSearchText(alias))) ? [id] : [],
  );
}

/** @param {string} normalized @param {string} needle */
function countOccurrences(normalized, needle) {
  if (needle.length === 0) return 0;
  let count = 0;
  let offset = 0;
  while ((offset = normalized.indexOf(needle, offset)) !== -1) {
    count += 1;
    offset += needle.length;
  }
  return count;
}

/** @param {string} normalized */
function lexicalFeatures(normalized) {
  /** @type {string[]} */
  const features = [...(normalized.match(/[a-z0-9.+-]{2,}/gu) ?? [])];
  const japanese = normalized.replace(/[^\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/gu, '');
  for (let index = 0; index + 1 < japanese.length; index += 1) {
    features.push(japanese.slice(index, index + 2));
  }
  return new Set(features);
}

/** @param {string} value */
function fnv1a(value) {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}
