extends Node3D

const TICK_RATE := 6.0
const CityScene = preload("res://city.gd")

var core: Object
var camera := Camera3D.new()
var city := CityScene.new()
var focus := Vector3(32, 0, 4)
var camera_size := 55.0
var paused := false
var pause_button: Button
var time_buffer := 0.0
var snapshot: Dictionary
var world: Dictionary
var status: Label
var guide_title: Label
var guide_text: Label
var guide_action: Button
var guide_card: PanelContainer
var guide_action_handler: Callable
var line_button: Button
var waiting_button: Button
var journeys_button: Button
var waypoint_button: Button
var stop_markers := {}
var selection_mark: Node3D
var explore_press := Vector2.ZERO
var explore_dragging := false
var explore_pressed := false
var marker_layer := Node3D.new()
var shows_waiting := false
var selected_lane := -1
var taught_street := false
var taught_stop := false
var shows_journeys := false
var has_journey_flows := false
var routes := {}
var waiting_markers := {}
var line_panel: HBoxContainer
var mode: OptionButton
var interval: SpinBox
var launch_button: Button
var drafting := false
var draft_stops: Array = []
var draft_legs: Array = []
var draft_marks := {}
var draft_result := {}
var hover_lane := -1
var dragged_leg := -1
var drag_start := Vector2.ZERO
var moving_stop := -1
var moving_waypoint := Vector2i(-1, -1)
var selected_source := -1
var editing_line := -1
var line_selector: OptionButton
var built_lines := {}
var waypoint_undo: Array = []

func _ready() -> void:
	_make_environment()
	_make_toolbar()
	if not ClassDB.class_exists("SimBridge"):
		GDExtensionManager.load_extension("res://sim_bridge.gdextension")
	if not ClassDB.class_exists("SimBridge"):
		status.text = "Die native Simulationsbrücke fehlt."
		return
	core = ClassDB.instantiate("SimBridge")
	world = core.world_data()
	if world.is_empty():
		status.text = "Die Brückenstadt konnte nicht geladen werden."
		return
	add_child(city)
	add_child(marker_layer)
	city.build(world.layout, world.lanes, world.stops)
	_refresh_snapshot()
	_show_explore()

func is_playable() -> bool:
	return not world.is_empty() and is_instance_valid(city) and city.is_inside_tree()

func _make_environment() -> void:
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color("#767f6b")
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.ambient_light_color = Color("#f3e8d9")
	environment.ambient_light_energy = 0.58
	var surroundings := WorldEnvironment.new()
	surroundings.environment = environment
	add_child(surroundings)

	var sun := DirectionalLight3D.new()
	sun.light_color = Color("#fff1d7")
	sun.light_energy = 0.83
	sun.rotation_degrees = Vector3(-51, -32, 0)
	sun.shadow_enabled = true
	add_child(sun)

	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	add_child(camera)
	camera.make_current()
	_update_camera()

func _make_toolbar() -> void:
	var layer := CanvasLayer.new()
	add_child(layer)
	_make_guide(layer)
	var margin := MarginContainer.new()
	layer.add_child(margin)
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_bottom", 16)
	margin.add_theme_constant_override("margin_left", 16)
	margin.add_theme_constant_override("margin_right", 16)
	margin.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var bar := VBoxContainer.new()
	bar.alignment = BoxContainer.ALIGNMENT_END
	margin.add_child(bar)
	bar.mouse_filter = Control.MOUSE_FILTER_IGNORE
	status = Label.new()
	status.add_theme_color_override("font_shadow_color", Color("#17251d"))
	status.add_theme_constant_override("shadow_offset_y", 2)
	status.add_theme_constant_override("outline_size", 5)
	status.add_theme_color_override("font_outline_color", Color("#17251d"))
	bar.add_child(status)
	var dock := PanelContainer.new()
	dock.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	dock.add_theme_stylebox_override("panel", _card_style())
	bar.add_child(dock)
	var dock_rows := VBoxContainer.new()
	dock_rows.add_theme_constant_override("separation", 9)
	dock.add_child(dock_rows)
	var tools := HBoxContainer.new()
	tools.mouse_filter = Control.MOUSE_FILTER_IGNORE
	tools.add_theme_constant_override("separation", 8)
	dock_rows.add_child(tools)
	line_panel = HBoxContainer.new()
	line_panel.add_theme_constant_override("separation", 8)
	dock_rows.add_child(line_panel)
	line_panel.hide()
	line_selector = OptionButton.new()
	line_selector.add_item("Neue Linie", 0)
	line_selector.item_selected.connect(_select_line)
	line_panel.add_child(line_selector)
	waypoint_button = Button.new()
	waypoint_button.text = "Wegpunkt entfernen"
	waypoint_button.tooltip_text = "Entfernt den zuletzt gesetzten Wegpunkt."
	waypoint_button.pressed.connect(_undo_waypoint)
	line_panel.add_child(waypoint_button)
	waypoint_button.hide()
	mode = OptionButton.new()
	mode.add_item("Bus")
	mode.add_item("Straßenbahn")
	mode.item_selected.connect(func(_which: int) -> void:
		_clear_draft()
		_update_stop_targets())
	line_panel.add_child(mode)
	var label := Label.new()
	label.text = "Abfahrt alle"
	line_panel.add_child(label)
	interval = SpinBox.new()
	interval.suffix = "Min."
	interval.min_value = 1
	interval.max_value = 60
	interval.value = 5
	interval.value_changed.connect(func(_value: float) -> void: _preview_line())
	line_panel.add_child(interval)
	launch_button = Button.new()
	launch_button.text = "Linie starten"
	launch_button.pressed.connect(_start_line)
	line_panel.add_child(launch_button)
	line_button = _toggle_button(tools, "Linie bauen", _set_drafting)
	line_button.tooltip_text = "Haltestellen setzen, Fahrweg prüfen und Takt wählen."
	line_button.add_theme_color_override("font_color", Color("#cbead3"))
	journeys_button = _toggle_button(tools, "Autowege", _set_journeys)
	journeys_button.tooltip_text = "Zeigt Herkunft und Ziele der Autos auf einer ausgewählten Straße."
	waiting_button = _toggle_button(tools, "Wartende", _set_waiting)
	waiting_button.tooltip_text = "Hebt wartende Menschen an Haltestellen hervor."
	var divider := VSeparator.new()
	tools.add_child(divider)
	pause_button = _toggle_button(tools, "Pause", func(_on: bool) -> void: toggle_pause())
	pause_button.tooltip_text = "Verkehr anhalten oder fortsetzen · Leertaste"
	var morning := Button.new()
	morning.text = "Nächster Morgen"
	morning.tooltip_text = "Springt zum nächsten Berufsverkehr. So kannst du deine Linie beobachten."
	morning.pressed.connect(_next_morning)
	tools.add_child(morning)
	var help := Button.new()
	help.text = "?"
	help.tooltip_text = "Bedienhinweise anzeigen oder ausblenden"
	help.pressed.connect(func() -> void: guide_card.visible = not guide_card.visible)
	tools.add_child(help)

func _card_style() -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = Color("#172823f2")
	style.border_color = Color("#546a5c")
	style.set_border_width_all(1)
	style.set_corner_radius_all(10)
	style.content_margin_left = 14
	style.content_margin_right = 14
	style.content_margin_top = 10
	style.content_margin_bottom = 10
	return style

func _make_guide(layer: CanvasLayer) -> void:
	guide_card = PanelContainer.new()
	layer.add_child(guide_card)
	guide_card.position = Vector2(16, 16)
	guide_card.custom_minimum_size.x = 360
	guide_card.size.x = 360
	guide_card.add_theme_stylebox_override("panel", _card_style())
	var content := VBoxContainer.new()
	content.add_theme_constant_override("separation", 6)
	guide_card.add_child(content)
	var heading := HBoxContainer.new()
	content.add_child(heading)
	guide_title = Label.new()
	guide_title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	guide_title.add_theme_font_size_override("font_size", 19)
	guide_title.add_theme_color_override("font_color", Color("#e9f2df"))
	heading.add_child(guide_title)
	var close := Button.new()
	close.text = "×"
	close.tooltip_text = "Hinweise ausblenden"
	close.flat = true
	close.pressed.connect(guide_card.hide)
	heading.add_child(close)
	guide_text = Label.new()
	guide_text.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	guide_text.add_theme_font_size_override("font_size", 14)
	guide_text.add_theme_color_override("font_color", Color("#cfdbce"))
	content.add_child(guide_text)
	guide_action = Button.new()
	guide_action.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	guide_action.pressed.connect(func() -> void:
		if guide_action_handler.is_valid():
			guide_action_handler.call())
	content.add_child(guide_action)

func _show_guide(title: String, text: String, action_text := "", action := Callable()) -> void:
	guide_title.text = title
	guide_text.text = text
	guide_action_handler = action
	guide_action.text = action_text
	guide_action.visible = not action_text.is_empty()
	guide_card.set_deferred("size", Vector2(360, 0))

func _show_explore() -> void:
	_show_guide("Erkunden",
		"Ziehe die Karte mit der Maus. Mit dem Mausrad zoomst du.\nViel Verkehr auf der Brücke? Klicke die Straße an und schau, wohin die Autos fahren.")

func _show_line_guide() -> void:
	if not drafting:
		return
	var place := "eine markierte Straße" if _kind() == "bus" else "ein markiertes Gleis"
	match draft_stops.size():
		0:
			_show_guide("Erster Halt", "Klicke auf " + place + ". Wenn du mit der Maus darüberfährst, leuchten die erreichbaren Häuser auf.")
		1:
			_show_guide("Zweiter Halt", "Setze einen weiteren Halt. Die Stadt zeichnet den passenden Fahrweg für dich.")
		_:
			if draft_result.get("ok", false):
				_show_guide("Linie prüfen", "Türkis zeigt den Fahrweg. Helle Häuser sind zu Fuß erreichbar. Ziehe den Fahrweg zum Ändern oder wähle unten die Abstände zwischen den Abfahrten und starte die Linie.")
			else:
				_show_guide("Fahrweg prüfen", "Die Verbindung ist gerade nicht befahrbar. Verschiebe einen Halt, entferne einen Wegpunkt oder nimm mit Esc den letzten Halt zurück.")

func _toggle_button(parent: Control, title: String, action: Callable) -> Button:
	var button := Button.new()
	button.text = title
	button.toggle_mode = true
	button.custom_minimum_size = Vector2(110, 38)
	button.toggled.connect(action)
	parent.add_child(button)
	return button

func toggle_pause() -> void:
	if not is_playable():
		return
	paused = not paused
	core.pause(paused)
	pause_button.set_pressed_no_signal(paused)
	pause_button.text = "Fortsetzen" if paused else "Pause"
	time_buffer = 0.0

func _next_morning() -> void:
	if not is_playable():
		return
	core.next_morning()
	time_buffer = 0.0
	_refresh_snapshot()

func _process(delta: float) -> void:
	if not is_playable():
		return
	if not paused:
		time_buffer += delta * TICK_RATE
		var ticks := mini(floori(time_buffer), 24)
		if ticks > 0:
			time_buffer -= ticks
			core.step(ticks)
			_refresh_snapshot()
	city.show_traffic(snapshot, 1.0 if paused else minf(1.0, time_buffer))

func _refresh_snapshot() -> void:
	snapshot = core.snapshot()
	if int(snapshot.get("revision", 0)) != int(world.get("revision", 0)):
		world = core.world_data()
		city.build(world.layout, world.lanes, world.stops)
		_clear_marks(routes)
		_clear_marks(waiting_markers)
		_preview_line()
		_update_stop_targets()
	var positioned_walkers := []
	for edge in snapshot.get("walkers", []):
		var endpoints := _foot_endpoints(int(edge.edge), int(edge.from), int(edge.to))
		if endpoints.size() == 2:
			positioned_walkers.append({"edge": edge.edge, "from": endpoints[0], "to": endpoints[1], "groups": edge.groups})
	snapshot.walkers = positioned_walkers
	_update_waiting()
	if shows_journeys:
		_update_journeys()
		if not drafting and guide_title.text == "Autowege":
			_show_journey_guide()

func _update_camera() -> void:
	camera.position = focus + Vector3(42, 55, 58)
	camera.look_at(focus)
	camera.size = camera_size

func _unhandled_input(event: InputEvent) -> void:
	if not is_playable():
		return
	if event is InputEventMouseMotion:
		var pan_buttons := MOUSE_BUTTON_MASK_MIDDLE | MOUSE_BUTTON_MASK_RIGHT
		if (event.button_mask & MOUSE_BUTTON_MASK_LEFT) == 0:
			explore_pressed = false
			if drafting:
				dragged_leg = -1
				moving_stop = -1
				moving_waypoint = Vector2i(-1, -1)
		if explore_pressed and event.position.distance_to(explore_press) > 6.0:
			explore_dragging = true
		if (event.button_mask & pan_buttons) != 0 or (explore_pressed and explore_dragging):
			var scale_per_pixel := camera_size / maxf(1.0, get_viewport().get_visible_rect().size.y)
			var right := camera.global_basis.x
			var forward := camera.global_basis.z.slide(Vector3.UP).normalized()
			focus += (-right * event.relative.x + forward * event.relative.y) * scale_per_pixel
			_update_camera()
		elif drafting:
			_update_hover(event.position)
	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_LEFT:
			if drafting:
				if event.pressed:
					_build_at(event.position)
				elif dragged_leg >= 0 or moving_stop >= 0 or moving_waypoint.x >= 0:
					_finish_drag(event.position)
			elif event.pressed:
				explore_press = event.position
				explore_pressed = true
				explore_dragging = false
			else:
				if explore_pressed and not explore_dragging:
					_inspect(event.position)
				explore_pressed = false
		elif event.pressed and event.button_index in [MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_DOWN]:
			_zoom(event.position, 0.87 if event.button_index == MOUSE_BUTTON_WHEEL_UP else 1.15)
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_SPACE:
			toggle_pause()
		elif event.keycode == KEY_ESCAPE:
			if drafting and not draft_stops.is_empty():
				draft_stops.pop_back()
				draft_legs.pop_back()
				_preview_line()
				return
			if drafting:
				_close_line_tool()
				return
			_select_street(-1)
			status.text = ""
			_show_explore()

func _ground(screen: Vector2) -> Vector3:
	var hit = Plane(Vector3.UP, 0).intersects_ray(camera.project_ray_origin(screen), camera.project_ray_normal(screen))
	return hit if hit != null else focus

func _zoom(screen: Vector2, factor: float) -> void:
	var before := _ground(screen)
	camera_size = clampf(camera_size * factor, 12, 88)
	_update_camera()
	focus += before - _ground(screen)
	_update_camera()

func _inspect(screen: Vector2) -> void:
	var point := _ground(screen)
	if shows_journeys and selected_lane >= 0:
		var origin := _origin_at(point)
		if origin >= 0:
			selected_source = -1 if selected_source == origin else origin
			_update_journeys()
			return
	for stop in snapshot.get("stops", []):
		if city.stop_position(int(stop.id)).distance_to(point) < 2.0:
			_select_street(-1)
			if not taught_stop:
				taught_stop = true
				_show_guide("Haltestelle", "Schau zu, wer einsteigt und wer auf das nächste Fahrzeug wartet.",
					"Wartende hervorheben", func() -> void: waiting_button.set_pressed(true))
			else:
				_show_guide("Haltestelle", "Hier halten Busse oder Straßenbahnen. Zoome näher heran, um die Wartenden zu sehen.")
			return
	_select_street(city.lane_at(point))
	if selected_lane < 0:
		_show_explore()
	elif shows_journeys:
		_show_journey_guide()
	elif not taught_street:
		taught_street = true
		_show_guide("Straße ausgewählt", "Woher kommen die Autos auf dieser Straße, und wohin fahren sie?",
			"Autowege anzeigen", func() -> void: journeys_button.set_pressed(true))
	else:
		_show_guide("Straße ausgewählt", "Mit „Autowege“ kannst du Herkunft und Ziele direkt auf der Karte sehen.")

func _select_street(lane: int) -> void:
	if lane != selected_lane:
		_clear_marks(routes)
	selected_lane = lane
	selected_source = -1
	if is_instance_valid(selection_mark):
		selection_mark.queue_free()
	if lane >= 0:
		selection_mark = _mark_curve(city.lane_curve(lane), Color("#fff2b5dd"), 0.44)
	_update_journeys()

func _set_waiting(on: bool) -> void:
	shows_waiting = on
	_update_waiting()
	if on and not drafting:
		_show_guide("Wartende an Haltestellen", "Gelbe Flächen heben die wartenden Gruppen hervor. Zoome näher heran, um zu sehen, wer einsteigt.")
	elif not drafting:
		_show_explore()

func _set_journeys(on: bool) -> void:
	shows_journeys = on
	_update_journeys()
	if not drafting:
		if on:
			_show_journey_guide()
		else:
			_show_explore()

func _show_journey_guide() -> void:
	var text := "Klicke auf eine Straße, deren Verkehr dich interessiert."
	if selected_lane >= 0:
		text = "Orange zeigt Herkunft, Blau das Ziel. Klicke auf einen orange markierten Wohnblock, um seine Wege einzeln zu verfolgen."
		if not has_journey_flows:
			text = "Gerade führt keine erfasste Autofahrt über diesen Abschnitt. Probiere eine andere Straße oder schau beim nächsten Berufsverkehr wieder vorbei."
	if guide_title.text != "Autowege" or guide_text.text != text:
		_show_guide("Autowege", text)

func _clear_marks(marks: Dictionary) -> void:
	for mark in marks.values():
		mark.queue_free()
	marks.clear()

func _ground_mark(color: Color) -> MeshInstance3D:
	var mark := MeshInstance3D.new()
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mark.material_override = material
	marker_layer.add_child(mark)
	return mark

func _update_waiting() -> void:
	if not shows_waiting or not is_playable():
		_clear_marks(waiting_markers)
		return
	for stop in snapshot.get("stops", []):
		var id := int(stop.id)
		if not waiting_markers.has(id):
			var mark := _ground_mark(Color("#f5d796cd"))
			var disc := CylinderMesh.new()
			disc.height = 0.035
			mark.mesh = disc
			waiting_markers[id] = mark
		var marker: MeshInstance3D = waiting_markers[id]
		marker.visible = int(stop.waiting) > 0
		marker.position = city.stop_position(id) + Vector3.UP * 0.14
		var width := 0.6 + minf(sqrt(float(stop.waiting)) * 0.27, 2.2)
		marker.scale = Vector3(width, 1, width)

func _update_journeys() -> void:
	has_journey_flows = false
	if not shows_journeys or selected_lane < 0:
		_clear_marks(routes)
		return
	var flows: Array = core.private_flows(selected_lane)
	has_journey_flows = not flows.is_empty()
	var links := {}
	var origins := {}
	var targets := {}
	var max_origin := 1
	for flow in flows:
		var start := int(flow.from)
		var destination := int(flow.to)
		origins[start] = int(origins.get(start, 0)) + int(flow.count)
		max_origin = maxi(max_origin, origins[start])
		if selected_source < 0 or selected_source == start:
			for lane in flow.lanes:
				links[int(lane)] = int(links.get(int(lane), 0)) + int(flow.count)
			targets[destination] = int(targets.get(destination, 0)) + int(flow.count)
	links[selected_lane] = maxi(int(links.get(selected_lane, 0)), 1)
	var wanted := {}
	for lane in links:
		wanted[Vector2i(0, lane)] = true
		if not routes.has(Vector2i(0, lane)):
			var color := Color("#f2d486d8") if lane == selected_lane else Color("#e39052af")
			routes[Vector2i(0, lane)] = _mark_curve(city.lane_curve(lane), color, 0.55)
	for building in world.layout.get("buildings", []):
		var zone := int(building.zone)
		if not origins.has(zone) and not targets.has(zone):
			continue
		var key := Vector2i(1, int(building.id))
		wanted[key] = true
		if not routes.has(key):
			var mark := _ground_mark(Color.WHITE)
			var footprint := BoxMesh.new()
			footprint.size = Vector3(6.8, 0.04, 6.8)
			mark.mesh = footprint
			mark.position = Vector3(float(building.cell.x) * 8, 0.19, float(building.cell.y) * 8)
			routes[key] = mark
		var marker: MeshInstance3D = routes[key]
		var weight := float(origins.get(zone, 0)) / max_origin
		var color := Color("#f3cc8590").lerp(Color("#ee8b41f0"), weight) if origins.has(zone) else Color("#a6cbecc0")
		if selected_source >= 0 and selected_source != zone and not targets.has(zone):
			color.a *= 0.37
		marker.material_override.albedo_color = color
	for key in routes.keys():
		if not wanted.has(key):
			routes[key].queue_free()
			routes.erase(key)

func _origin_at(point: Vector3) -> int:
	var from_zones := {}
	for flow in core.private_flows(selected_lane):
		from_zones[int(flow.from)] = true
	for house in world.layout.buildings:
		var center := Vector3(float(house.cell.x) * 8, 0, float(house.cell.y) * 8)
		if from_zones.has(int(house.zone)) and center.distance_to(point) <= 3.5:
			return int(house.zone)
	return -1

func _mark_curve(curve: PackedVector3Array, color: Color, width: float) -> Node3D:
	var line := Node3D.new()
	marker_layer.add_child(line)
	for index in range(1, curve.size()):
		var start := curve[index - 1]
		var finish := curve[index]
		if start.is_equal_approx(finish):
			continue
		var mesh := BoxMesh.new()
		mesh.size = Vector3(width, 0.045, start.distance_to(finish) + 0.12)
		var strip := _ground_mark(color)
		strip.reparent(line, false)
		strip.mesh = mesh
		strip.position = (start + finish) * 0.5 + Vector3.UP * 0.18
		strip.look_at(finish + Vector3.UP * 0.18)
	return line

func _set_drafting(on: bool) -> void:
	drafting = on
	explore_pressed = false
	line_panel.visible = on
	line_button.text = "Planung schließen" if on else "Linie bauen"
	if on:
		guide_card.show()
	_update_stop_targets()
	if on:
		_select_line(line_selector.selected)
	else:
		_clear_draft()
	status.text = ""
	if not on:
		_show_explore()

func _close_line_tool() -> void:
	line_button.set_pressed_no_signal(false)
	_set_drafting(false)

func _update_stop_targets() -> void:
	_clear_marks(stop_markers)
	if not drafting or not is_playable():
		return
	for spec in world.lanes:
		var lane := int(spec.id)
		if _kind() in spec.allowed and _stop_zone(lane) >= 0:
			stop_markers[lane] = _mark_curve(city.lane_curve(lane), Color("#ffe8a476"), 0.65)

func _clear_draft() -> void:
	moving_waypoint = Vector2i(-1, -1)
	draft_stops.clear()
	draft_legs.clear()
	waypoint_undo.clear()
	hover_lane = -1
	dragged_leg = -1
	moving_stop = -1
	_preview_line()

func _kind() -> String:
	return "bus" if mode.selected == 0 else "tram"

func _allowed_lane(point: Vector3) -> int:
	var best := 3.0
	var selected := -1
	for spec in world.lanes:
		if not _kind() in spec.allowed:
			continue
		var curve: PackedVector3Array = city.lane_curve(int(spec.id))
		for i in range(1, curve.size()):
			var flat := Vector3(point.x, curve[i].y, point.z)
			var nearest := Geometry3D.get_closest_point_to_segment(flat, curve[i - 1], curve[i])
			var distance := nearest.distance_to(flat)
			if distance < best:
				best = distance
				selected = int(spec.id)
	return selected

func _stop_zone(lane: int) -> int:
	for item in world.stops:
		if int(item.lane) == lane:
			return int(item.zone)
	for zone in world.zones:
		if zone.road_access != null and int(zone.road_access) == lane:
			return int(zone.id)
	return -1

func _update_hover(screen: Vector2) -> void:
	var lane := _allowed_lane(_ground(screen))
	if lane >= 0 and _stop_zone(lane) < 0:
		lane = -1
	if lane != hover_lane:
		hover_lane = lane
		_redraw_draft()

func _build_at(screen: Vector2) -> void:
	var point := _ground(screen)
	var lane := _allowed_lane(point)
	if lane < 0:
		status.text = "Hier ist kein Halt für dieses Verkehrsmittel möglich."
		return
	drag_start = screen
	for index in draft_stops.size():
		var stop: Dictionary = draft_stops[index]
		if int(stop.lane) == lane and city.point_on_lane(lane, 0.9).distance_to(point) < 2.3:
			moving_stop = index
			return
	for leg_index in draft_legs.size():
		for pin_index in draft_legs[leg_index].size():
			var via := int(draft_legs[leg_index][pin_index])
			if city.point_on_lane(via, 0.5).distance_to(point) < 2.3:
				moving_waypoint = Vector2i(leg_index, pin_index)
				return
	var preview_routes: Array = draft_result.get("routes", [])
	for index in mini(preview_routes.size(), draft_stops.size() - 1):
		if lane in preview_routes[index]:
			dragged_leg = index
			return
	_add_stop(lane)

func _add_stop(lane: int) -> void:
	if _stop_zone(lane) < 0:
		status.text = "An diesem Abschnitt fehlt ein Fußzugang."
		return
	if draft_stops.any(func(stop: Dictionary) -> bool: return int(stop.lane) == lane):
		return
	draft_stops.append({"lane": lane, "zone": _stop_zone(lane)})
	draft_legs.append([])
	_preview_line()

func _finish_drag(screen: Vector2) -> void:
	var lane := _allowed_lane(_ground(screen))
	var moved := screen.distance_to(drag_start) > 7
	var leg := dragged_leg
	var stop := moving_stop
	var waypoint := moving_waypoint
	dragged_leg = -1
	moving_stop = -1
	moving_waypoint = Vector2i(-1, -1)
	if lane < 0:
		return
	if waypoint.x >= 0:
		if moved:
			draft_legs[waypoint.x][waypoint.y] = lane
			_preview_line()
	elif stop >= 0:
		if moved and _stop_zone(lane) >= 0 and not draft_stops.any(func(item: Dictionary) -> bool: return int(item.lane) == lane):
			draft_stops[stop] = {"lane": lane, "zone": _stop_zone(lane)}
			_preview_line()
	elif moved and not lane in draft_result.routes[leg]:
		draft_legs[leg].append(lane)
		waypoint_undo.append(leg)
		_preview_line()
	elif not moved:
		_add_stop(lane)

func _preview_line() -> void:
	draft_result = {}
	if is_playable() and draft_stops.size() >= 2:
		draft_result = core.line_edit(editing_line, _kind(), draft_stops, draft_legs, int(interval.value), false)
		status.text = "" if draft_result.get("ok", false) else str(draft_result.get("error", ""))
	launch_button.disabled = not draft_result.get("ok", false)
	waypoint_button.visible = draft_legs.any(func(leg: Array) -> bool: return not leg.is_empty())
	_show_line_guide()
	if is_playable():
		_redraw_draft()

func _redraw_draft() -> void:
	_clear_marks(draft_marks)
	if not drafting:
		return
	for route in draft_result.get("routes", []):
		for lane in route:
			var key := int(lane)
			if not draft_marks.has(key):
				draft_marks[key] = _mark_curve(city.lane_curve(key), Color("#8be0c2d8"), 0.72)
	var candidates := draft_stops.duplicate()
	if hover_lane >= 0 and not candidates.any(func(stop: Dictionary) -> bool: return int(stop.lane) == hover_lane):
		candidates.append({"lane": hover_lane, "zone": _stop_zone(hover_lane)})
	_draw_access(candidates)
	for index in draft_stops.size():
		var key := "stop" + str(index)
		_draft_pin(key, int(draft_stops[index].lane), Color("#7ee4c8"))
		var label := Label3D.new()
		label.text = str(index + 1)
		label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
		label.no_depth_test = true
		label.font_size = 45
		label.pixel_size = 0.027
		label.outline_size = 10
		label.position = Vector3.UP * 1.1
		draft_marks[key].add_child(label)
	for leg in draft_legs.size():
		for pin in draft_legs[leg].size():
			var color := Color("#a8d8f6") if draft_result.get("ok", false) else Color("#eda972")
			_draft_pin("via_%d_%d" % [leg, pin], int(draft_legs[leg][pin]), color, 0.5)
	if hover_lane >= 0:
		_draft_pin("hover", hover_lane, Color("#d9f5b6a8"))

func _draw_access(candidates: Array) -> void:
	if candidates.is_empty():
		return
	var entries: Array = core.walking_access(int(candidates.front().zone), true)
	var exits: Array = [] if candidates.size() < 2 else core.walking_access(int(candidates.back().zone), false)
	var relevant := {}
	if selected_lane >= 0 and not exits.is_empty():
		var accessible_starts := {}
		var accessible_ends := {}
		for path in entries:
			accessible_starts[int(path.zone)] = true
		for path in exits:
			accessible_ends[int(path.zone)] = true
		for flow in core.private_flows(selected_lane):
			if accessible_starts.has(int(flow.from)) and accessible_ends.has(int(flow.to)):
				relevant[int(flow.from)] = true
				relevant[int(flow.to)] = true
	for path in entries + exits:
		var zone := int(path.zone)
		var color := Color("#f7cf79d2") if relevant.has(zone) else Color("#a6e1b078")
		_mark_access_houses(zone, color)
		for edge in path.legs:
			var key := "walk_" + str(edge[0]) + "_" + str(edge[1])
			if draft_marks.has(key):
				continue
			var endpoints := _foot_endpoints(int(edge[2]), int(edge[0]), int(edge[1]))
			if endpoints.size() == 2:
				draft_marks[key] = _mark_curve(endpoints, color, 0.23)

func _mark_access_houses(zone: int, color: Color) -> void:
	for house in world.layout.buildings:
		if int(house.zone) != zone:
			continue
		var key := "access_" + str(house.id)
		if draft_marks.has(key):
			continue
		var marker := _ground_mark(color)
		var plate := BoxMesh.new()
		plate.size = Vector3(6.5, 0.03, 6.5)
		marker.mesh = plate
		marker.position = Vector3(float(house.cell.x) * 8, 0.16, float(house.cell.y) * 8)
		draft_marks[key] = marker

func _foot_endpoints(edge: int, from: int, to: int) -> PackedVector3Array:
	var start = _zone_position(from)
	var finish = _zone_position(to)
	if start == null or finish == null or start.is_equal_approx(finish):
		return PackedVector3Array()
	for street in world.layout.get("streets", []):
		if not edge in street.get("foot_edges", []):
			continue
		var curve: PackedVector3Array = city.lane_curve(int(street.lane))
		if curve.size() < 2:
			break
		var first: Vector3 = curve[0]
		var last: Vector3 = curve[-1]
		var side := (last - first).normalized().cross(Vector3.UP)
		var offset := 1.12
		for lane in world.lanes:
			if int(lane.id) == int(street.lane) and "tram" in lane.allowed:
				offset = 0.89
				break
		first -= side * offset
		last -= side * offset
		if start.distance_to(last) + finish.distance_to(first) < start.distance_to(first) + finish.distance_to(last):
			var prior := first
			first = last
			last = prior
			side = -side
		var pavement := side * 3.08 + Vector3.UP * 0.08
		return PackedVector3Array([first + pavement, last + pavement])
	return PackedVector3Array([start, finish])

func _zone_position(zone: int) -> Variant:
	for stop in draft_stops:
		if int(stop.zone) == zone:
			return city.point_on_lane(int(stop.lane), 0.9)
	for stop in world.stops:
		if int(stop.zone) == zone:
			return city.point_on_lane(int(stop.lane), 0.9)
	for house in world.layout.buildings:
		if int(house.zone) == zone:
			return Vector3(float(house.cell.x) * 8, 0, float(house.cell.y) * 8)
	return null

func _draft_pin(key: String, lane: int, color: Color, fraction := 0.9) -> void:
	var mark := _ground_mark(color)
	var mesh := CylinderMesh.new()
	mesh.top_radius = 0.44
	mesh.bottom_radius = 0.6
	mesh.height = 1.45
	mark.mesh = mesh
	mark.position = city.point_on_lane(lane, fraction) + Vector3.UP * 0.82
	draft_marks[key] = mark

func _start_line() -> void:
	var result: Dictionary = core.line_edit(editing_line, _kind(), draft_stops, draft_legs, int(interval.value), true)
	if not result.get("ok", false):
		status.text = str(result.error)
		return
	var id := int(result.line)
	if not built_lines.has(id):
		line_selector.add_item("Linie " + str(built_lines.size() + 1), id)
	built_lines[id] = {
		"kind": mode.selected, "minutes": interval.value,
		"stops": draft_stops.duplicate(true), "legs": draft_legs.duplicate(true),
	}
	world = core.world_data()
	city.build(world.layout, world.lanes, world.stops)
	_refresh_snapshot()
	line_selector.select(0)
	_select_line(0)
	_close_line_tool()
	_show_guide("Linie fährt", "Schau an einer Haltestelle zu. Beim nächsten Berufsverkehr kannst du sehen, wie sich die Verbindung auf den Verkehr auswirkt.",
		"Zum nächsten Morgen", _next_morning)

func _select_line(index: int) -> void:
	editing_line = -1 if index == 0 else line_selector.get_item_id(index)
	mode.disabled = editing_line >= 0
	launch_button.text = "Linie starten" if editing_line < 0 else "Änderungen übernehmen"
	_clear_draft()
	if editing_line >= 0:
		var saved: Dictionary = built_lines[editing_line]
		mode.select(int(saved.kind))
		interval.set_value_no_signal(float(saved.minutes))
		draft_stops = saved.stops.duplicate(true)
		draft_legs = saved.legs.duplicate(true)
		_preview_line()

func _undo_waypoint() -> void:
	while not waypoint_undo.is_empty():
		var leg: int = waypoint_undo.pop_back()
		if leg < draft_legs.size() and not draft_legs[leg].is_empty():
			draft_legs[leg].pop_back()
			_preview_line()
			return
	for index in range(draft_legs.size() - 1, -1, -1):
		if not draft_legs[index].is_empty():
			draft_legs[index].pop_back()
			_preview_line()
			return
