type Line = { indent: number; text: string; raw: string; blank: boolean };

type BlockHeader = { style: '|' | '>'; chomp: 'clip' | 'strip' | 'keep'; indent: number | null };

function blockHeader(rest: string): BlockHeader | null {
  if (rest[0] !== '|' && rest[0] !== '>') return null;
  let i = 1;
  let chomp: BlockHeader['chomp'] = 'clip';
  let indent: number | null = null;
  const takeChomp = () => {
    if (rest[i] === '-') { chomp = 'strip'; i++; return true; }
    if (rest[i] === '+') { chomp = 'keep'; i++; return true; }
    return false;
  };
  const takeIndent = () => {
    if (rest[i] >= '1' && rest[i] <= '9') { indent = Number(rest[i]); i++; return true; }
    return false;
  };
  if (!takeChomp()) takeIndent();
  if (indent === null) takeIndent();
  else takeChomp();
  if (i !== rest.length) return null;
  return { style: rest[0], chomp, indent };
}

function foldScalar(text: string): string {
  const endsNl = text.endsWith('\n');
  const lines = text.split('\n');
  if (endsNl) lines.pop();
  let out = '';
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (i > 0) {
      const prev = lines[i - 1];
      const broken = prev === '' || line === '' || line.startsWith(' ') || prev.startsWith(' ');
      out += broken ? '\n' : ' ';
    }
    out += line;
  }
  return endsNl ? `${out}\n` : out;
}

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
      if (q === "'" && s[i] === "'" && s[i + 1] === "'") { out += "'"; i += 2; continue; }
      if (s[i] === q) break;
      if (q === '"' && s[i] === '\\') { i++; out += s[i] === 'n' ? '\n' : (s[i] ?? ''); i++; continue; }
      out += s[i++];
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
  const lines: Line[] = src.split('\n').map((raw) => {
    const blank = raw.trim() === '';
    return { indent: blank ? 0 : raw.length - raw.trimStart().length, text: raw.trim(), raw, blank };
  });
  let pos = 0;

  const skipBlanks = () => { while (pos < lines.length && lines[pos].blank) pos++; };
  const node = (indent: number): unknown => (lines[pos].text.startsWith('- ') ? seq(indent) : map(indent));

  function blockScalar(header: BlockHeader, parentIndent: number): string {
    const explicit = header.indent === null ? null : parentIndent + header.indent;
    const parts: string[] = [];
    let contentIndent = explicit;
    while (pos < lines.length) {
      const line = lines[pos];
      if (!line.blank) {
        if (contentIndent === null) contentIndent = line.indent;
        if (line.indent < contentIndent) break;
      }
      pos++;
      if (line.blank) parts.push('');
      else parts.push(line.raw.slice(Math.min(contentIndent ?? line.indent, line.raw.length)));
    }
    let text = parts.join('\n');
    if (header.chomp === 'strip') text = text.replace(/\n+$/, '');
    else if (header.chomp === 'clip') text = `${text.replace(/\n+$/, '')}\n`;
    else text += '\n';
    return header.style === '>' ? foldScalar(text) : text;
  }

  function valueOf(rest: string, indent: number): unknown {
    const header = blockHeader(rest.trim());
    return header ? blockScalar(header, indent) : parseScalar(rest);
  }

  function map(indent: number): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    while (true) {
      skipBlanks();
      const line = lines[pos];
      if (!line || line.indent !== indent || line.text.startsWith('- ')) break;
      const m = KEY.exec(line.text);
      if (!m) throw new Error(`cannot parse line: ${line.text}`);
      pos++;
      const [, key, rest] = m;
      if (rest !== undefined && rest.trim() !== '') out[key] = valueOf(rest, indent);
      else {
        skipBlanks();
        const next = lines[pos];
        if (next && (next.indent > indent || (next.indent === indent && next.text.startsWith('- ')))) out[key] = node(next.indent);
        else out[key] = null;
      }
    }
    return out;
  }

  function seq(indent: number): unknown[] {
    const out: unknown[] = [];
    while (true) {
      skipBlanks();
      const line = lines[pos];
      if (!line || line.indent !== indent || !line.text.startsWith('- ')) break;
      const item = line.text.slice(2);
      if (KEY.test(item)) {
        lines[pos] = { ...line, indent: indent + 2, text: item };
        out.push(map(indent + 2));
      } else {
        pos++;
        out.push(item.trim() ? valueOf(item, indent) : null);
      }
    }
    return out;
  }

  skipBlanks();
  if (pos >= lines.length) return {};
  const r = node(lines[pos].indent);
  skipBlanks();
  if (pos < lines.length) throw new Error(`unexpected indentation at: ${lines[pos].text}`);
  return r as Record<string, unknown>;
}
