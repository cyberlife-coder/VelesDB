import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SearchBar } from './components/SearchBar';
import { Results } from './components/Results';
import { Ingest } from './components/Ingest';
import { Database, Zap, FileText } from 'lucide-react';

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

interface IndexStats {
  total_chunks: number;
  dimension: number;
}

function App() {
  const [results, setResults] = useState<SearchResult | null>(null);
  const [stats, setStats] = useState<IndexStats | null>(null);
  const [activeTab, setActiveTab] = useState<'search' | 'ingest'>('search');

  const refreshStats = async () => {
    try {
      const s = await invoke<IndexStats>('get_stats');
      setStats(s);
    } catch (err) {
      console.error('Failed to get stats:', err);
    }
  };

  useEffect(() => {
    refreshStats();
  }, []);

  return (
    <div className="min-h-screen bg-gradient-to-br from-slate-900 to-slate-800 text-white">
      <header className="border-b border-slate-700 bg-slate-900/50 backdrop-blur-sm">
        <div className="max-w-4xl mx-auto px-4 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <Database className="w-8 h-8 text-blue-400" />
              <div>
                <h1 className="text-xl font-bold">VelesDB RAG</h1>
                <p className="text-sm text-slate-400">Local vector search in microseconds</p>
              </div>
            </div>
            {stats && (
              <div className="flex items-center gap-4 text-sm">
                <div className="flex items-center gap-2">
                  <FileText className="w-4 h-4 text-slate-400" />
                  <span>{stats.total_chunks} chunks</span>
                </div>
                <div className="flex items-center gap-2">
                  <Zap className="w-4 h-4 text-yellow-400" />
                  <span>{stats.dimension}D vectors</span>
                </div>
              </div>
            )}
          </div>
        </div>
      </header>

      <main className="max-w-4xl mx-auto px-4 py-8">
        <div className="mb-6">
          <div className="flex gap-2">
            <button
              onClick={() => setActiveTab('search')}
              className={`px-4 py-2 rounded-lg font-medium transition-colors ${
                activeTab === 'search'
                  ? 'bg-blue-500 text-white'
                  : 'bg-slate-700 text-slate-300 hover:bg-slate-600'
              }`}
            >
              Search
            </button>
            <button
              onClick={() => setActiveTab('ingest')}
              className={`px-4 py-2 rounded-lg font-medium transition-colors ${
                activeTab === 'ingest'
                  ? 'bg-blue-500 text-white'
                  : 'bg-slate-700 text-slate-300 hover:bg-slate-600'
              }`}
            >
              Ingest
            </button>
          </div>
        </div>

        {activeTab === 'search' ? (
          <div className="space-y-6">
            <SearchBar onResults={setResults} />
            {results && <Results results={results} />}
          </div>
        ) : (
          <Ingest onComplete={refreshStats} />
        )}
      </main>

      <footer className="fixed bottom-0 left-0 right-0 border-t border-slate-700 bg-slate-900/80 backdrop-blur-sm">
        <div className="max-w-4xl mx-auto px-4 py-3 text-center text-sm text-slate-400">
          Powered by VelesDB — Vector Search in Microseconds
        </div>
      </footer>
    </div>
  );
}

export default App;
