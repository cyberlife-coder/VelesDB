"""``import velesdb`` stays zero-dep: no embedding backend is imported until
an adapter is built."""

import importlib
import sys

import pytest

BACKENDS = ("openai", "sentence_transformers")


def test_importing_the_embed_module_imports_no_backend(monkeypatch):
    attempted = []

    class Recorder:
        def find_spec(self, name, path=None, target=None):
            if name.split(".")[0] in BACKENDS:
                attempted.append(name)
            return None

    for name in BACKENDS:
        monkeypatch.delitem(sys.modules, name, raising=False)
    monkeypatch.setattr(sys, "meta_path", [Recorder(), *sys.meta_path])

    import velesdb.embed as embed

    importlib.reload(embed)
    assert attempted == [], f"imported at module load: {attempted}"


def test_a_missing_backend_names_its_extra(monkeypatch):
    import velesdb.embed as embed

    monkeypatch.setitem(sys.modules, "openai", None)  # import fails
    with pytest.raises(ImportError, match="embed-openai"):
        embed.OpenAIEmbedder()
