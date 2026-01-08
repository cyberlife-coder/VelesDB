import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { SearchBar } from './components/SearchBar';
import { Results } from './components/Results';
import { Ingest } from './components/Ingest';
import { Zap, FileText, Loader2, Sparkles } from 'lucide-react';
import velesdbIcon from './assets/velesdb-icon.png';

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

interface ModelStatus {
  loaded: boolean;
  model_name: string;
  dimension: number;
}

// Demo data for VelesDB showcase
const DEMO_TEXT = `# VelesDB - Lightning-Fast Vector Database

VelesDB is a high-performance vector database designed for AI applications, offering microsecond-level search latency.

## Key Features

### Ultra-Fast Search
VelesDB uses the HNSW (Hierarchical Navigable Small World) algorithm optimized with SIMD instructions, achieving search times under 1 millisecond for millions of vectors.

### Multiple Distance Metrics
VelesDB supports Cosine Similarity for normalized embeddings and semantic search, Euclidean Distance for spatial data and image features, and Dot Product for maximum inner product search.

### Enterprise Features
VelesDB provides persistent storage with crash recovery, real-time indexing without downtime, horizontal scaling support, and REST API with native Rust bindings.

## Performance Benchmarks

Tested on standard hardware with 16-core CPU and 64GB RAM, VelesDB achieves 0.5ms p99 latency for 100K vectors with 99.2% recall, 2.1ms for 1M vectors with 98.8% recall, and 8.5ms for 10M vectors with 98.1% recall.

## Use Cases

### Semantic Search
Power your search engine with semantic understanding. VelesDB finds relevant documents based on meaning, not just keywords.

### RAG Applications
Retrieve the most relevant context for your LLM in microseconds, enabling real-time conversational AI with Retrieval-Augmented Generation.

### Recommendation Systems
Find similar items, users, or content using vector similarity for personalized recommendations across e-commerce, streaming, and social platforms.

## Comparison with Competitors

VelesDB is 50-100x faster than cloud alternatives like Pinecone. While Pinecone has 45ms p50 latency, VelesDB achieves 0.89ms. VelesDB is also completely free to self-host, compared to $70-700/month for cloud solutions.`;

function App() {
  const [results, setResults] = useState<SearchResult | null>(null);
  const [stats, setStats] = useState<IndexStats | null>(null);
  const [activeTab, setActiveTab] = useState<'search' | 'ingest'>('search');
  const [modelStatus, setModelStatus] = useState<ModelStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingStatus, setLoadingStatus] = useState('Initializing...');

  const refreshStats = async () => {
    try {
      const s = await invoke<IndexStats>('get_stats');
      setStats(s);
    } catch (err) {
      console.error('Failed to get stats:', err);
    }
  };

  const loadDemoData = async () => {
    try {
      setLoadingStatus('Loading demo data...');
      await invoke('ingest_text', { text: DEMO_TEXT, chunkSize: 500 });
      await refreshStats();
    } catch (err) {
      console.error('Failed to load demo data:', err);
    }
  };

  useEffect(() => {
    const init = async () => {
      try {
        // Check model status
        setLoadingStatus('Checking embedding model...');
        const status = await invoke<ModelStatus>('get_model_status');
        setModelStatus(status);

        if (!status.loaded) {
          // Preload the model (downloads ~90MB on first run)
          setLoadingStatus('Loading ML model (first run may download ~90MB)...');
          await invoke('preload_model');
          setModelStatus({ ...status, loaded: true });
        }

        setLoadingStatus('Ready!');
        await refreshStats();
        
        // Small delay for UX
        await new Promise(r => setTimeout(r, 500));
        setLoading(false);
      } catch (err) {
        console.error('Initialization error:', err);
        setLoadingStatus(`Error: ${err}`);
        // Still show the app after error
        setTimeout(() => setLoading(false), 2000);
      }
    };

    init();
  }, []);

  // Splash screen during model loading
  if (loading) {
    return (
      <div className="min-h-screen bg-gradient-to-br from-dark-950 via-dark-900 to-dark-800 flex flex-col items-center justify-center text-white">
        <img src={velesdbIcon} alt="VelesDB" className="w-24 h-24 mb-6 animate-bounce" />
        <h1 className="text-3xl font-bold mb-2">VelesDB RAG Demo</h1>
        <p className="text-dark-300 mb-8">Semantic Search with Real ML Embeddings</p>
        <Loader2 className="w-8 h-8 animate-spin text-primary-400 mb-4" />
        <p className="text-dark-400 text-sm">{loadingStatus}</p>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gradient-to-br from-dark-950 via-dark-900 to-dark-800 text-white">
      <header className="border-b border-dark-800 bg-dark-900/70 backdrop-blur-sm shadow-lg shadow-primary-500/5">
        <div className="max-w-4xl mx-auto px-4 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-3">
              <img src={velesdbIcon} alt="VelesDB" className="w-8 h-8" />
              <div>
                <h1 className="text-xl font-bold">VelesDB RAG</h1>
                <p className="text-sm text-dark-300">Semantic search with {modelStatus?.model_name || 'ML'}</p>
              </div>
            </div>
            <div className="flex items-center gap-4">
              {stats && stats.total_chunks === 0 && (
                <button
                  onClick={loadDemoData}
                  className="px-3 py-1.5 bg-primary-500 hover:from-primary-500/90 hover:to-accent-500/90 bg-gradient-to-r from-primary-500 to-accent-500 rounded-lg text-sm font-medium transition-colors flex items-center gap-2 shadow-md shadow-primary-500/30"
                >
                  <Sparkles className="w-4 h-4" />
                  Load Demo
                </button>
              )}
              {stats && (
                <div className="flex items-center gap-4 text-sm">
                  <div className="flex items-center gap-2">
                    <FileText className="w-4 h-4 text-dark-300" />
                    <span>{stats.total_chunks} chunks</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <Zap className="w-4 h-4 text-yellow-300" />
                    <span>{stats.dimension}D vectors</span>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      </header>

      {/* Performance badge */}
      <div className="bg-gradient-to-r from-primary-900/20 via-dark-900/40 to-accent-900/20 border-b border-dark-800">
        <div className="max-w-4xl mx-auto px-4 py-2 text-center text-sm">
          <span className="text-yellow-400">⚡</span>
          <span className="text-dark-100 ml-2">50-100x faster than cloud alternatives</span>
          <span className="text-dark-500 mx-2">|</span>
          <span className="text-emerald-300">$0 self-hosted</span>
          <span className="text-dark-500 mx-2">|</span>
          <span className="text-primary-200">100% offline</span>
        </div>
      </div>

      <main className="max-w-4xl mx-auto px-4 py-8">
        <div className="mb-6">
          <div className="flex gap-2">
            <button
              onClick={() => setActiveTab('search')}
              className={`px-4 py-2 rounded-lg font-medium transition-colors ${
                activeTab === 'search'
                  ? 'bg-gradient-to-r from-primary-600 to-primary-500 text-white shadow-glow'
                  : 'bg-dark-800 text-dark-200 hover:bg-dark-700'
              }`}
            >
              Search
            </button>
            <button
              onClick={() => setActiveTab('ingest')}
              className={`px-4 py-2 rounded-lg font-medium transition-colors ${
                activeTab === 'ingest'
                  ? 'bg-gradient-to-r from-primary-600 to-primary-500 text-white shadow-glow'
                  : 'bg-dark-800 text-dark-200 hover:bg-dark-700'
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
            {!results && stats && stats.total_chunks > 0 && (
              <div className="text-center py-8 text-dark-400">
                <p>Try searching: "What is VelesDB?" or "How fast is semantic search?"</p>
              </div>
            )}
            {stats && stats.total_chunks === 0 && (
              <div className="text-center py-12 text-dark-400">
                <p className="mb-4">No documents indexed yet.</p>
                <button
                  onClick={loadDemoData}
                  className="px-4 py-2 bg-gradient-to-r from-primary-500 to-accent-500 hover:from-primary-400 hover:to-accent-400 rounded-lg font-medium transition-colors inline-flex items-center gap-2 shadow-glow"
                >
                  <Sparkles className="w-4 h-4" />
                  Load Demo Data
                </button>
              </div>
            )}
          </div>
        ) : (
          <Ingest onComplete={refreshStats} />
        )}
      </main>

      <footer className="border-t border-dark-800 bg-dark-950/80 backdrop-blur-sm mt-12">
        <div className="max-w-4xl mx-auto px-4 py-3 text-center text-sm text-dark-300 flex items-center justify-center gap-2">
          <img src={velesdbIcon} alt="VelesDB" className="w-4 h-4" />
          <span>Powered by VelesDB — Vector Search in Microseconds</span>
        </div>
      </footer>
    </div>
  );
}

export default App;
