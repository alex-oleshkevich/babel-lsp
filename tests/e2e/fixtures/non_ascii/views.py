from flask_babel import _


def greet():
    # Non-ASCII string literal and message IDs
    return _("Héllo wörld")


def cjk():
    return _("日本語テスト")


def rtl():
    return _("مرحبا بالعالم")
