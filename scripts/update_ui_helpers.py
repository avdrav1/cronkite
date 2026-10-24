#!/usr/bin/env python3
"""Read the update setting and notice in an app's windows, through AT-SPI (UPD-3).

    update_ui_helpers.py APP text         every name, description, and text in APP's
                                          windows, one a line
    update_ui_helpers.py APP windows      the names of APP's windows, one a line
    update_ui_helpers.py APP press LABEL  activate the button named LABEL
    update_ui_helpers.py APP item LABEL   the menu item or toggle button showing LABEL:
                                          "checked" or "unchecked" ("none": no such item)

APP is the program's name ("brick"). A popover menu item has no name of its own: the
label it's labelled by holds its text. Used by scripts/check_update_ui.sh; needs
python-gobject and at-spi2-core, inside the app's D-Bus session.
"""

import sys

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402


def app(name):
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        a = desktop.get_child_at_index(i)
        if a is not None and a.get_name() == name:
            return a
    sys.exit(f"no accessible app named {name!r}")


def walk(node, depth=0):
    if node is None or depth > 60:
        return
    node.clear_cache()
    yield node
    for i in range(node.get_child_count()):
        yield from walk(node.get_child_at_index(i), depth + 1)


def label(node):
    """A node's name, else the name of the label it's labelled by."""
    if node.get_name():
        return node.get_name()
    for r in node.get_relation_set():
        if r.get_relation_type() == Atspi.RelationType.LABELLED_BY:
            for i in range(r.get_n_targets()):
                t = r.get_target(i)
                text = t.get_text_iface() if t is not None else None
                if text is not None:
                    return Atspi.Text.get_text(text, 0, Atspi.Text.get_character_count(text))
    return ""


def main():
    name, cmd, *rest = sys.argv[1:]
    root = app(name)
    if cmd == "text":
        for node in walk(root):
            if node.get_name():
                print(node.get_name())
            if node.get_description():
                print(node.get_description())
            text = node.get_text_iface()
            if text is not None:
                content = Atspi.Text.get_text(text, 0, Atspi.Text.get_character_count(text))
                if content and content != node.get_name():
                    print(content)
    elif cmd == "windows":
        for i in range(root.get_child_count()):
            w = root.get_child_at_index(i)
            if w is not None:
                print(w.get_name())
    elif cmd == "press":
        for node in walk(root):
            if node.get_role() == Atspi.Role.BUTTON and node.get_name() == rest[0]:
                Atspi.Action.do_action(node.get_action_iface(), 0)
                return
        sys.exit(f"no button {rest[0]!r}")
    elif cmd == "item":
        for node in walk(root):
            role = node.get_role()
            items = (Atspi.Role.MENU_ITEM, Atspi.Role.CHECK_MENU_ITEM, Atspi.Role.TOGGLE_BUTTON)
            if role in items and label(node) == rest[0]:
                # A toggle button says it's on as pressed, a check item as checked.
                states = node.get_state_set()
                on = states.contains(Atspi.StateType.CHECKED) or states.contains(
                    Atspi.StateType.PRESSED
                )
                print("checked" if on else "unchecked")
                return
        print("none")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
