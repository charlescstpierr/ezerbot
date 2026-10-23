import { useMemo, useState, useRef } from 'react';
import { Mail, FileSpreadsheet, FileText } from 'lucide-react';
import { Streamdown, defaultRehypePlugins } from 'streamdown';
import { citationPlugin, sourceDetails, openSource } from './sources.js';

export function SourcesProse({ message, botId, children, ...props }) {
  const sources = message.payload?.sources || [];
  const citations = message.payload?.citations || [];
  const [selected, select] = useState(null);
  const [result, setResult] = useState('');
  const trigger = useRef(null);
  const plugins = useMemo(() => [...Object.values(defaultRehypePlugins), citationPlugin(citations)], [citations]);
  const choose = (source, event) => { trigger.current = event.currentTarget; select(source); setResult(''); };
  const close = () => { select(null); trigger.current?.focus(); };
  const citation = ({ 'data-source': number, children }) => {
    const source = sources.find(item => item.n === Number(number));
    return source && !citations.some(item => item.n === source.n && item.etat === 'introuvable')
      ? <button type="button" className="gb-citation" title={sourceDetails(source)} aria-haspopup="dialog" onClick={event => choose(source, event)}>{children}</button>
      : <span className="gb-citation gb-source-missing" title="Source introuvable" tabIndex={0} aria-label={`Source ${number} introuvable`}>{children}</span>;
  };
  return <>
    <Streamdown {...props} rehypePlugins={plugins} components={{ button: citation }}>{children}</Streamdown>
    {sources.length > 0 && <details className="gb-sources"><summary><span className="gb-source-icons" aria-hidden="true"><Mail /><FileSpreadsheet /><FileText /></span>Sources ({sources.length})</summary><ol>{sources.map(source => <li key={source.n}><button type="button" onClick={event => choose(source, event)}>[{source.n}] {sourceDetails(source)}</button></li>)}</ol></details>}
    {selected && <section className="gb-source-card" role="dialog" aria-label={`Source ${selected.n}`} onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); close(); } }}>
      <small>{selected.type.replaceAll('_', ' ')}</small><strong>{selected.titre}</strong>
      <p>{[selected.auteur, selected.date, selected.plage].filter(Boolean).join(' · ')}</p>
      <button type="button" autoFocus onClick={async () => setResult(await openSource(selected, { botId: message.payload?.bot_id || message.bot_id || botId }))}>Ouvrir la source</button>{' '}
      <button type="button" onClick={close}>Fermer</button>
      {typeof result === 'string' ? result && <p role="status">{result}</p> : <pre tabIndex={0}>{result.text}</pre>}
    </section>}
  </>;
}
