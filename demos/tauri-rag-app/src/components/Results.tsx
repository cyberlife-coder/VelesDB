import { Zap, FileText } from 'lucide-react';

interface Chunk {
  id: number;
  text: string;
  score?: number;
}

interface SearchResult {
  chunks: Chunk[];
  query: string;
  time_ms: number;
}

interface ResultsProps {
  results: SearchResult;
}

export function Results({ results }: ResultsProps) {
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between text-sm text-slate-400">
        <span>
          Found {results.chunks.length} relevant chunks for "{results.query}"
        </span>
        <span className="flex items-center gap-1">
          <Zap className="w-4 h-4 text-yellow-400" />
          {results.time_ms.toFixed(2)}ms
        </span>
      </div>

      <div className="space-y-3">
        {results.chunks.map((chunk, index) => (
          <div
            key={chunk.id}
            className="p-4 bg-slate-800 border border-slate-700 rounded-lg hover:border-slate-600 transition-colors"
          >
            <div className="flex items-start justify-between gap-4">
              <div className="flex items-center gap-2 text-sm text-slate-400">
                <FileText className="w-4 h-4" />
                <span>Chunk #{chunk.id}</span>
              </div>
              {chunk.score !== undefined && (
                <span className="px-2 py-1 text-xs font-medium bg-blue-500/20 text-blue-400 rounded">
                  {(chunk.score * 100).toFixed(1)}% match
                </span>
              )}
            </div>
            <p className="mt-2 text-slate-200 leading-relaxed">
              {chunk.text}
            </p>
            <div className="mt-3 text-xs text-slate-500">
              Rank #{index + 1}
            </div>
          </div>
        ))}
      </div>

      {results.chunks.length === 0 && (
        <div className="text-center py-8 text-slate-400">
          No results found. Try ingesting some documents first.
        </div>
      )}
    </div>
  );
}
