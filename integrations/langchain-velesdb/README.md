# langchain-velesdb

LangChain integration for [VelesDB](https://github.com/cyberlife-coder/VelesDB) vector database.

## Installation

```bash
pip install langchain-velesdb
```

## Quick Start

```python
from langchain_velesdb import VelesDBVectorStore
from langchain_openai import OpenAIEmbeddings

# Initialize vector store
vectorstore = VelesDBVectorStore(
    path="./my_vectors",
    collection_name="documents",
    embedding=OpenAIEmbeddings()
)

# Add documents
vectorstore.add_texts([
    "VelesDB is a high-performance vector database",
    "Built entirely in Rust for speed and safety",
    "Perfect for RAG applications and semantic search"
])

# Search
results = vectorstore.similarity_search("fast database", k=2)
for doc in results:
    print(doc.page_content)
```

## Usage with RAG

```python
from langchain_velesdb import VelesDBVectorStore
from langchain_openai import ChatOpenAI, OpenAIEmbeddings
from langchain.chains import RetrievalQA

# Create vector store with documents
vectorstore = VelesDBVectorStore.from_texts(
    texts=["Document 1 content", "Document 2 content"],
    embedding=OpenAIEmbeddings(),
    path="./rag_data",
    collection_name="knowledge_base"
)

# Create RAG chain
retriever = vectorstore.as_retriever(search_kwargs={"k": 3})
qa_chain = RetrievalQA.from_chain_type(
    llm=ChatOpenAI(),
    chain_type="stuff",
    retriever=retriever
)

# Ask questions
answer = qa_chain.run("What is VelesDB?")
print(answer)
```

## API Reference

### VelesDBVectorStore

```python
VelesDBVectorStore(
    embedding: Embeddings,
    path: str = "./velesdb_data",
    collection_name: str = "langchain",
    metric: str = "cosine"  # "cosine", "euclidean", "dot"
)
```

#### Methods

- `add_texts(texts, metadatas=None, ids=None)` - Add texts to the store
- `similarity_search(query, k=4)` - Search for similar documents
- `similarity_search_with_score(query, k=4)` - Search with similarity scores
- `delete(ids)` - Delete documents by ID
- `as_retriever(**kwargs)` - Convert to LangChain retriever
- `from_texts(texts, embedding, ...)` - Create store from texts (class method)

## Features

- **High Performance**: VelesDB's Rust backend delivers microsecond latencies
- **SIMD Optimized**: Hardware-accelerated vector operations
- **Simple Setup**: Single binary, no external dependencies
- **Full LangChain Compatibility**: Works with all LangChain chains and agents

## License

Business Source License 1.1 (BSL-1.1)

See [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE) for details.
