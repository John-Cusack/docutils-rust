"""`docutils_rust`: Python face of the docutils-rust port.

Thin shim over the compiled `_core` extension (PyO3). `publish_string` and
`publish_doctree` mirror the `docutils.core` entry points for the
reStructuredText parser, the transform pipeline, and the HTML publisher
implemented in Rust.

This is a clean-room port of a subset of `docutils`; it is not a drop-in
replacement for the upstream package (whose import name stays with
upstream).
"""

from docutils_rust._core import publish_doctree as _publish_doctree
from docutils_rust._core import publish_string as _publish_string
from docutils_rust._core import version as _version

__version__ = _version()

__all__ = ["publish_string", "publish_doctree", "__version__"]


def publish_string(source, writer_name="html5", settings_overrides=None):
    """Publish `source` and return `(output, messages)`.

    Mirrors `docutils.core.publish_string`: `writer_name` is one of
    `html5` / `pseudoxml` / `null`, and `settings_overrides` accepts
    `report_level`, `halt_level`, and `writer_name`. `messages` is a list
    of `(level, text, line)` triples at or above `report_level`.
    """
    return _publish_string(source, writer_name, settings_overrides)


def publish_doctree(source, settings_overrides=None):
    """Parse `source` and return `(doctree, messages)`.

    Mirrors `docutils.core.publish_doctree` structurally: blocks become
    `{"type": ...}` dicts so callers can walk the tree without the CPython
    node classes.
    """
    return _publish_doctree(source, settings_overrides)
