"""Finder presentation only; installation remains a normal app copy."""

files = [defines["app"]]
symlinks = {"Applications": "/Applications"}
format = "UDZO"
filesystem = "HFS+"
background = defines["background"]
window_rect = ((120, 120), (640, 280))
icon_locations = {"Craft Apps Manager.app": (140, 120), "Applications": (500, 120)}
default_view = "icon-view"
icon_size = 96
text_size = 13
show_icon_preview = False
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
include_icon_view_settings = True
include_list_view_settings = False
