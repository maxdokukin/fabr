type Line = { indent: number; text: string };

export function parseScalar(raw: string): unknown {
  const s = raw.trim();
  if (s === '' || s === 'null' || s === '~') return null;
  if (s === 'true') return true;
  if (s === 'false') return false;
  if (s[0] === '"' || s[0] === "'" || s[0] === '{' || s[0] === '[') return parseFlow(s);
  if (/^-?\d+(\.\d+)?$/.test(s)) return Number(s);
  return s;
}

function parseFlow(s: string): unknown {
  let i = 0;
  const ws = () => { while (s[i] === ' ') i++; };
  const quoted = (): string => {
    const q = s[i++];
    let out = '';
    while (i < s.length) {
      if (q === "'" && s[i] === "'" && s[i + 1] === "'") { out += "'"; i += 2; }
      else if (s[i] === q) break;
      else if (q === '"' && s[i] === '\\') { i++; out += s[i] === 'n' ? '\n' : s[i]; i++; }
      else out += s[i++];
    }
    if (s[i] !== q) throw new Error(`unterminated string in: ${s}`);
    i++;
    return out;
  };
  const plain = (stops: string): string => {
    const st = i;
    while (i < s.length && !stops.includes(s[i])) i++;
    return s.slice(st, i).trim();
  };
  const value = (stops: string): unknown => {
    ws();
    if (s[i] === '"' || s[i] === "'") return quoted();
    if (s[i] === '{') return map();
    if (s[i] === '[') return list();
    return parseScalar(plain(stops));
  };
  const list = (): unknown[] => {
    i++;
    const out: unknown[] = [];
    ws();
    while (i < s.length && s[i] !== ']') {
      out.push(value(',]'));
      ws();
      if (s[i] === ',') i++;
      ws();
    }
    if (s[i] !== ']') throw new Error(`unterminated list in: ${s}`);
    i++;
    return out;
  };
  const map = (): Record<string, unknown> => {
    i++;
    const out: Record<string, unknown> = {};
    ws();
    while (i < s.length && s[i] !== '}') {
      ws();
      const key = s[i] === '"' || s[i] === "'" ? quoted() : plain(':');
      ws();
      if (s[i] !== ':') throw new Error(`expected ':' after "${key}" in: ${s}`);
      i++;
      out[key] = value(',}');
      ws();
      if (s[i] === ',') i++;
      ws();
    }
    if (s[i] !== '}') throw new Error(`unterminated map in: ${s}`);
    i++;
    return out;
  };
  const v = value('');
  ws();
  if (i < s.length) throw new Error(`unexpected text after value in: ${s}`);
  return v;
}

const KEY = /^([A-Za-z_][\w-]*):(?: (.*))?$/;

export function parseYaml(src: string): Record<string, unknown> {
  const lines: Line[] = [];
  for (const l of src.split('\n')) {
    if (!l.trim()) continue;
    lines.push({ indent: l.length - l.trimStart().length, text: l.trim() });
  }
  let pos = 0;

  const node = (indent: number): unknown => (lines[pos].text.startsWith('- ') ? seq(indent) : map(indent));

  function map(indent: number): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    while (pos < lines.length && lines[pos].indent === indent && !lines[pos].text.startsWith('- ')) {
      const m = KEY.exec(lines[pos].text);
      if (!m) throw new Error(`cannot parse line: ${lines[pos].text}`);
      pos++;
      const [, key, rest] = m;
      if (rest !== undefined && rest.trim() !== '') out[key] = parseScalar(rest);
      else {
        const next = lines[pos];
        if (next && (next.indent > indent || (next.indent === indent && next.text.startsWith('- ')))) out[key] = node(next.indent);
        else out[key] = null;
      }
    }
    return out;
  }

  function seq(indent: number): unknown[] {
    const out: unknown[] = [];
    while (pos < lines.length && lines[pos].indent === indent && lines[pos].text.startsWith('- ')) {
      const item = lines[pos].text.slice(2);
      if (KEY.test(item)) {
        lines[pos] = { indent: indent + 2, text: item };
        out.push(map(indent + 2));
      } else {
        pos++;
        out.push(parseScalar(item));
      }
    }
    return out;
  }

  if (!lines.length) return {};
  const r = node(lines[0].indent);
  if (pos < lines.length) throw new Error(`unexpected indentation at: ${lines[pos].text}`);
  return r as Record<string, unknown>;
}
