import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Upload, Loader2, CheckCircle, FileText, Trash2 } from 'lucide-react';

interface Chunk {
  id: number;
  text: string;
  score?: number;
}

interface IngestProps {
  onComplete: () => void;
}

const SAMPLE_TEXT = `# VelesDB Overview

VelesDB is a high-performance vector database designed for AI applications.
It provides microsecond-level search latency, making it ideal for real-time RAG.

## Key Features

VelesDB supports multiple distance metrics including cosine similarity, 
Euclidean distance, and dot product. It uses SIMD optimizations for 
maximum performance on modern CPUs.

## Use Cases

VelesDB is perfect for semantic search, recommendation systems, and 
retrieval-augmented generation (RAG) applications. It works entirely 
offline, ensuring data privacy.

## Performance

With VelesDB, you can search through 100,000 vectors in under 50 
milliseconds. Single vector insertions take only 2-3 microseconds.`;

export function Ingest({ onComplete }: IngestProps) {
  const [text, setText] = useState(SAMPLE_TEXT);
  const [loading, setLoading] = useState(false);
  const [chunks, setChunks] = useState<Chunk[]>([]);
  const [error, setError] = useState<string | null>(null);

  const handleIngest = async () => {
    if (!text.trim()) return;

    setLoading(true);
    setError(null);

    try {
      const result = await invoke<Chunk[]>('ingest_text', { 
        text, 
        chunkSize: 500 
      });
      setChunks(result);
      onComplete();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleClear = async () => {
    setLoading(true);
    try {
      await invoke('clear_index');
      setChunks([]);
      onComplete();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <label className="block text-sm font-medium text-dark-200 mb-2">
          Paste your text content
        </label>
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={10}
          placeholder="Paste your document text here..."
          className="w-full px-4 py-3 bg-dark-900/70 border border-dark-700 rounded-lg text-white placeholder-dark-400 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:border-transparent transition-all resize-none font-mono text-sm"
        />
      </div>

      <div className="flex gap-3">
        <button
          onClick={handleIngest}
          disabled={loading || !text.trim()}
          className="flex-1 px-6 py-3 bg-gradient-to-r from-primary-600 to-primary-500 hover:from-primary-500 hover:to-primary-400 disabled:bg-dark-700 disabled:text-dark-500 disabled:cursor-not-allowed text-white font-medium rounded-lg transition-colors flex items-center justify-center gap-2 shadow-md shadow-primary-500/20"
        >
          {loading ? (
            <>
              <Loader2 className="w-5 h-5 animate-spin" />
              Processing...
            </>
          ) : (
            <>
              <Upload className="w-5 h-5" />
              Ingest Document
            </>
          )}
        </button>
        <button
          onClick={handleClear}
          disabled={loading}
          className="px-6 py-3 bg-rose-500/10 hover:bg-rose-500/20 text-rose-300 font-medium rounded-lg transition-colors flex items-center gap-2"
        >
          <Trash2 className="w-5 h-5" />
          Clear Index
        </button>
      </div>

      {error && (
        <p className="text-rose-400 text-sm">{error}</p>
      )}

      {chunks.length > 0 && (
        <div className="space-y-3">
          <div className="flex items-center gap-2 text-emerald-300">
            <CheckCircle className="w-5 h-5" />
            <span>Successfully ingested {chunks.length} chunks</span>
          </div>
          <div className="space-y-2">
            {chunks.map((chunk) => (
              <div
                key={chunk.id}
                className="p-3 bg-dark-900/60 border border-dark-800 rounded-lg text-sm"
              >
                <div className="flex items-center gap-2 text-dark-300 mb-1">
                  <FileText className="w-4 h-4" />
                  <span>Chunk #{chunk.id}</span>
                </div>
                <p className="text-dark-100 line-clamp-2">
                  {chunk.text}
                </p>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
