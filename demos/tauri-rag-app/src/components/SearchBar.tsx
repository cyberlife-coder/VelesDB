import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Search, Loader2 } from 'lucide-react';

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

interface SearchBarProps {
  onResults: (results: SearchResult) => void;
}

export function SearchBar({ onResults }: SearchBarProps) {
  const [query, setQuery] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSearch = async () => {
    if (!query.trim()) return;

    setLoading(true);
    setError(null);

    try {
      const results = await invoke<SearchResult>('search', { query, k: 5 });
      onResults(results);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      handleSearch();
    }
  };

  return (
    <div className="space-y-2">
      <div className="flex gap-2">
        <div className="relative flex-1">
          <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-5 h-5 text-dark-400" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Ask a question about your documents..."
            className="w-full pl-10 pr-4 py-3 bg-dark-900/70 border border-dark-700 rounded-lg text-white placeholder-dark-400 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all"
          />
        </div>
        <button
          onClick={handleSearch}
          disabled={loading || !query.trim()}
          className="px-6 py-3 bg-gradient-to-r from-primary-600 to-primary-500 hover:from-primary-500 hover:to-primary-400 disabled:bg-dark-700 disabled:text-dark-500 disabled:cursor-not-allowed text-white font-medium rounded-lg transition-colors flex items-center gap-2 shadow-md shadow-primary-500/20"
        >
          {loading ? (
            <>
              <Loader2 className="w-5 h-5 animate-spin" />
              Searching...
            </>
          ) : (
            <>
              <Search className="w-5 h-5" />
              Search
            </>
          )}
        </button>
      </div>
      {error && (
        <p className="text-rose-400 text-sm">{error}</p>
      )}
    </div>
  );
}
