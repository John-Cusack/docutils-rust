Inline Markup
=============

Emphasis *one*, strong **two**, literal ``three``.

Nested **strong with *emphasis* inside** done.

Escaped \*not emphasis\* and \\ backslash.

Interpreted `bare title` here.

:emphasis:`explicit em`, :strong:`explicit st`, :literal:`explicit lit`.

:code:`some_code()`, :title-reference:`Work Title`.

:pep-reference:`8`, :rfc-reference:`1234`.

.. role:: custom(emphasis)

.. default-role:: strong

Bare `defaulted` role text.

.. Unknown role :nosuch:`x` here.

Unknown target: missing-link_ here.

>>> doctest example
>>> 1 + 1
2

A *star alone and a trailing_ underscore word_ boundary test.

Email in text: first.last+tag@example.co.uk end.
