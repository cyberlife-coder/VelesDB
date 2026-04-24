"""Typed exceptions for the LlamaIndex VelesDB integration.

The exceptions in this module are specific to the integration layer
so callers (and integration test suites) can catch capability gaps
surgically rather than reacting to the broad built-in hierarchy.

Backward compatibility: :class:`VelesDBCapabilityError` inherits
from :class:`NotImplementedError` (not :class:`RuntimeError`) so
existing code that catches ``NotImplementedError`` — the exception
the integration raised before Sprint 1.5 — continues to work
without modification. New code should prefer the typed variant to
access the :attr:`capability` attribute for introspection.
"""

from __future__ import annotations


class VelesDBCapabilityError(NotImplementedError):
    """Raised when the backing VelesDB collection does not expose a
    capability the LlamaIndex integration relies on.

    The attribute :attr:`capability` carries the name of the missing
    method (for example ``"search_with_filter"``) so callers that want
    to fall back to a degraded mode can branch on it. The exception
    message embeds a ``remediation`` hint explaining how the operator
    can restore the capability (typically by recreating the collection
    as a vector collection instead of a legacy type).

    Inheritance: this exception derives from
    :class:`NotImplementedError` so legacy code that uses
    ``except NotImplementedError:`` still catches it. The previous
    integration behaviour (before Sprint 1.5) raised a plain
    ``NotImplementedError`` directly; the switch to a typed subclass
    is a strict superset — no existing catch block breaks.
    """

    def __init__(self, capability: str, remediation: str = "") -> None:
        message = f"VelesDB collection does not support capability '{capability}'."
        if remediation:
            message = f"{message} {remediation}"
        super().__init__(message)
        self.capability = capability
