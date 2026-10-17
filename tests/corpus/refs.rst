References
==========

.. _named-target: https://example.com/named

.. _`quoted target`: https://example.com/quoted

.. __: https://example.com/anon1

.. __: https://example.com/anon2

.. _indirect: named-target_

.. _internal-anchor:

A named-target_ link and `quoted target`_ link.

An anonymous link__ and another__.

An indirect_ link and internal-anchor_ link.

A phrase `text <https://example.com/phrase>`_ and `mail <me@example.org>`_.

Standalone https://example.org/standalone and me@example.org mail.

Footnote one [1]_, auto [#]_, named [#note]_, symbol [*]_.

Citation here [CITE]_.

|sub-para| and |sub-img| follow.

.. [1] Manual footnote body.

.. [#] Auto footnote body.

.. [#note] Named auto footnote body.

.. [*] Symbol footnote body.

.. [CITE] Citation body text.

.. |sub-para| replace:: replaced *text* here

.. |sub-img| image:: https://example.com/i.png
   :alt: img alt

.. |sym| unicode:: U+0041 U+0042
