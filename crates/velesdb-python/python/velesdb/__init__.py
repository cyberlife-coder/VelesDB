"""VelesDB Python Bindings.

The Rust extension remains the source of truth. For older prebuilt extension
artifacts that do not expose VelesQL symbols yet, we provide a lightweight
fallback parser so `from velesdb import VelesQL` remains import-compatible.
"""

from velesdb import velesdb as _core

Database = _core.Database
Collection = _core.Collection
SearchResult = _core.SearchResult
FusionStrategy = _core.FusionStrategy
GraphStore = _core.GraphStore
StreamingConfig = _core.StreamingConfig
TraversalResult = _core.TraversalResult
__version__ = _core.__version__

# Prefer native classes from Rust extension when available.
VelesQL = getattr(_core, "VelesQL", None)
ParsedStatement = getattr(_core, "ParsedStatement", None)
VelesQLSyntaxError = getattr(_core, "VelesQLSyntaxError", None)
VelesQLParameterError = getattr(_core, "VelesQLParameterError", None)


if VelesQL is None or ParsedStatement is None:
    class VelesQLSyntaxError(Exception):
        """Raised when a VelesQL query has syntax errors."""


    class VelesQLParameterError(Exception):
        """Raised when VelesQL query parameters are invalid."""


    class ParsedStatement:
        def __init__(self, query: str):
            import re

            self._query = query.strip()
            self._upper = self._query.upper()

            m = re.search(r"\bFROM\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s+([A-Za-z_][A-Za-z0-9_]*))?", self._query, re.IGNORECASE)
            self._table = m.group(1) if m else None
            self._alias = m.group(2) if m else None

            self._distinct = bool(re.search(r"^\s*SELECT\s+DISTINCT\b", self._query, re.IGNORECASE))
            self._where = bool(re.search(r"\bWHERE\b", self._upper))
            self._joins = len(re.findall(r"\bJOIN\b", self._upper))
            self._group = bool(re.search(r"\bGROUP\s+BY\b", self._upper))
            self._having = bool(re.search(r"\bHAVING\b", self._upper))
            self._fusion = bool(re.search(r"\bUSING\s+FUSION\b", self._upper))
            self._vector = bool(re.search(r"\bVECTOR\s+NEAR\b", self._upper)) or "SIMILARITY(" in self._upper
            self._order = bool(re.search(r"\bORDER\s+BY\b", self._upper))

            lm = re.search(r"\bLIMIT\s+(\d+)", self._upper)
            self._limit = int(lm.group(1)) if lm else None
            om = re.search(r"\bOFFSET\s+(\d+)", self._upper)
            self._offset = int(om.group(1)) if om else None

            cols_m = re.search(r"^\s*SELECT\s+(.*?)\s+FROM\b", self._query, re.IGNORECASE | re.DOTALL)
            if cols_m:
                cols_raw = cols_m.group(1).strip()
                if cols_raw.upper().startswith("DISTINCT "):
                    cols_raw = cols_raw[9:].strip()
                if cols_raw == "*":
                    self._cols = ["*"]
                else:
                    self._cols = [c.strip() for c in cols_raw.split(",") if c.strip()]
            else:
                self._cols = []

            ob_m = re.search(r"\bORDER\s+BY\s+(.*?)(?:\bLIMIT\b|\bOFFSET\b|$)", self._query, re.IGNORECASE | re.DOTALL)
            self._order_by = []
            if ob_m:
                for part in [p.strip() for p in ob_m.group(1).split(",") if p.strip()]:
                    pieces = part.split()
                    field = pieces[0]
                    direction = pieces[1].upper() if len(pieces) > 1 and pieces[1].upper() in ("ASC", "DESC") else "ASC"
                    self._order_by.append((field, direction))

            gb_m = re.search(r"\bGROUP\s+BY\s+(.*?)(?:\bHAVING\b|\bORDER\b|\bLIMIT\b|\bOFFSET\b|$)", self._query, re.IGNORECASE | re.DOTALL)
            self._group_by = [p.strip() for p in gb_m.group(1).split(",")] if gb_m else []

        @property
        def table_name(self):
            return self._table

        @property
        def table_alias(self):
            return self._alias

        @property
        def columns(self):
            return self._cols

        @property
        def limit(self):
            return self._limit

        @property
        def offset(self):
            return self._offset

        @property
        def order_by(self):
            return self._order_by

        @property
        def group_by(self):
            return self._group_by

        @property
        def join_count(self):
            return self._joins

        def is_valid(self):
            return True

        def is_select(self):
            return self._upper.startswith("SELECT ")

        def is_match(self):
            return self._upper.startswith("MATCH ")

        def has_distinct(self):
            return self._distinct

        def has_where_clause(self):
            return self._where

        def has_order_by(self):
            return self._order

        def has_group_by(self):
            return self._group

        def has_having(self):
            return self._having

        def has_joins(self):
            return self._joins > 0

        def has_fusion(self):
            return self._fusion

        def has_vector_search(self):
            return self._vector

        def __repr__(self):
            query_type = "SELECT" if self.is_select() else "MATCH"
            table = self._table or "<none>"
            return f"ParsedStatement({query_type} FROM {table})"

        def __str__(self):
            lines = [
                f"Type: {'SELECT' if self.is_select() else 'MATCH'}",
                f"Table: {self._table or '<none>'}",
            ]
            if self._limit is not None:
                lines.append(f"LIMIT: {self._limit}")
            if self._offset is not None:
                lines.append(f"OFFSET: {self._offset}")
            return "\n".join(lines)


    class VelesQL:
        @staticmethod
        def parse(query: str):
            import re

            q = query.strip()
            upper = q.upper()
            if not upper.startswith(("SELECT ", "MATCH ")):
                raise VelesQLSyntaxError("VelesQL syntax error at position 0: query must start with SELECT or MATCH")
            if upper.startswith("SELECT ") and not re.search(r"\bFROM\b", upper):
                raise VelesQLSyntaxError("VelesQL syntax error at position 7: missing FROM clause")
            if upper.startswith("SELECT ") and re.match(r"^\s*SELECT\s+FROM\b", upper):
                raise VelesQLSyntaxError("VelesQL syntax error at position 7: missing projection")
            return ParsedStatement(q)

        @staticmethod
        def is_valid(query: str) -> bool:
            try:
                VelesQL.parse(query)
                return True
            except VelesQLSyntaxError:
                return False

__all__ = [
    "Database",
    "Collection",
    "SearchResult",
    "FusionStrategy",
    "GraphStore",
    "StreamingConfig",
    "TraversalResult",
    "VelesQL",
    "ParsedStatement",
    "VelesQLSyntaxError",
    "VelesQLParameterError",
    "__version__",
]
