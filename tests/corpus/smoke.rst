Sample Guide
============

.. contents:: Table of contents

This is an *introductory* paragraph with **bold** text, ``literal`` code,
a `link <https://example.com>`_, and a standalone https://example.org/ URL.

Section One
-----------

- First bullet with *markup*
- Second bullet

  Continued paragraph in the same item.

- Third bullet

1. First step
2. Second step

Term One
   Definition of term one spanning
   multiple lines.

Term Two
   Definition of term two.

:field: value
:other: more

::

   literal block content
   second line

.. note::

   A note admonition.

.. warning:: Custom warning title

   Warning body text.

.. code:: python

   def f(x):
       return x * 2

+-----+-----+
| a   | b   |
+=====+=====+
| 1   | 2   |
+-----+-----+

Footnote ref [1]_ and auto [#]_ plus citation [CIT]_.

.. [1] First footnote.
.. [#] Auto footnote.
.. [CIT] A citation.

.. _target-name: https://example.com/target

See target-name_ for details.

|release| is the current version.

.. |release| replace:: 1.2.3

----

Closing paragraph with an :emphasis:`explicit role`.
