extends Node3D

const TICK_RATE := 6.0
const CityScene = preload("res://city.gd")

var core: Object
var camera := Camera3D.new()
var city := CityScene.new()
var focus := Vector3(32, 0, 4)
var camera_size := 55.0
var paused := false
var time_buffer := 0.0
var snapshot: Dictionary
var world: Dictionary
var status: Label
var marker_layer := Node3D.new()
var shows_waiting := false
var selected_lane := -1
var taught_street := false
var taught_stop := false
var shows_journeys := false
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
	var tools := HBoxContainer.new()
	tools.mouse_filter = Control.MOUSE_FILTER_IGNORE
	tools.add_theme_constant_override("separation", 8)
	bar.add_child(tools)
	line_panel = HBoxContainer.new()
	line_panel.add_theme_constant_override("separation", 8)
	bar.add_child(line_panel)
	line_panel.hide()
	line_selector = OptionButton.new()
	line_selector.add_item("Neue Linie", 0)
	line_selector.item_selected.connect(_select_line)
	line_panel.add_child(line_selector)
	var remove_waypoint := Button.new()
	remove_waypoint.text = "Letzten Wegpunkt entfernen"
	remove_waypoint.pressed.connect(_undo_waypoint)
	line_panel.add_child(remove_waypoint)
	mode = OptionButton.new()
	mode.add_item("Bus")
	mode.add_item("Straßenbahn")
	mode.item_selected.connect(func(_which: int) -> void:
		_clear_draft()
		status.text = "Wähle einen passenden Halt auf der Karte.")
	line_panel.add_child(mode)
	var label := Label.new()
	label.text = "Takt (Minuten)"
	line_panel.add_child(label)
	interval = SpinBox.new()
	interval.min_value = 1
	interval.max_value = 60
	interval.value = 5
	interval.value_changed.connect(func(_value: float) -> void: _preview_line())
	line_panel.add_child(interval)
	launch_button = Button.new()
	launch_button.text = "Linie starten"
	launch_button.pressed.connect(_start_line)
	line_panel.add_child(launch_button)
	_toggle_button(tools, "Linie bauen", _set_drafting)
	_toggle_button(tools, "Pause", func(_on: bool) -> void: toggle_pause())
	_toggle_button(tools, "Wartende", _set_waiting)
	_toggle_button(tools, "Wege anzeigen", _set_journeys)

func _toggle_button(parent: Control, title: String, action: Callable) -> void:
	var button := Button.new()
	button.text = title
	button.toggle_mode = true
	button.custom_minimum_size = Vector2(110, 38)
	button.toggled.connect(action)
	parent.add_child(button)

func toggle_pause() -> void:
	if not is_playable():
		return
	paused = not paused
	core.pause(paused)
	time_buffer = 0.0

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
	_update_waiting()
	if shows_journeys:
		_update_journeys()

func _update_camera() -> void:
	camera.position = focus + Vector3(42, 55, 58)
	camera.look_at(focus)
	camera.size = camera_size

func _unhandled_input(event: InputEvent) -> void:
	if not is_playable():
		return
	if event is InputEventMouseMotion and (event.button_mask & MOUSE_BUTTON_MASK_MIDDLE) != 0:
		var direction: Vector3 = -camera.global_basis.x * event.relative.x + camera.global_basis.z * event.relative.y
		direction.y = 0.0
		focus += direction * camera_size / maxf(1.0, get_viewport().get_visible_rect().size.y)
		_update_camera()
	if event is InputEventMouseMotion and drafting:
		_update_hover(event.position)
	if event is InputEventMouseButton and not event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		if dragged_leg >= 0 or moving_stop >= 0:
			_finish_drag(event.position)
	if event is InputEventMouseButton and event.pressed:
		match event.button_index:
			MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_DOWN:
				_zoom(event.position, 0.87 if event.button_index == MOUSE_BUTTON_WHEEL_UP else 1.15)
			MOUSE_BUTTON_LEFT:
				if drafting:
					_build_at(event.position)
				else:
					_inspect(event.position)
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_SPACE:
			toggle_pause()
		elif event.keycode == KEY_ESCAPE:
			if drafting and not draft_stops.is_empty():
				draft_stops.pop_back()
				draft_legs.pop_back()
				_preview_line()
				return
			selected_lane = -1
			_clear_marks(routes)
			status.text = ""

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
			if not taught_stop:
				status.text = "Mit „Wartende“ siehst du die Gruppen auch aus der Ferne."
				taught_stop = true
			return
	selected_lane = city.lane_at(point)
	selected_source = -1
	if selected_lane < 0:
		status.text = ""
	elif not taught_street:
		status.text = "Mit „Wege anzeigen“ kannst du Autofahrten zurückverfolgen."
		taught_street = true
	else:
		status.text = ""
	_update_journeys()

func _set_waiting(on: bool) -> void:
	shows_waiting = on
	_update_waiting()

func _set_journeys(on: bool) -> void:
	shows_journeys = on
	if on and selected_lane < 0:
		status.text = "Wähle eine Straße auf der Karte."
	_update_journeys()

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
	if not shows_journeys or selected_lane < 0:
		_clear_marks(routes)
		return
	var flows: Array = core.private_flows(selected_lane)
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
	line_panel.visible = on
	if on:
		_select_line(line_selector.selected)
	else:
		_clear_draft()
	status.text = "Wähle eine Straße oder ein Gleis für den ersten Halt." if on else ""

func _clear_draft() -> void:
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
	dragged_leg = -1
	moving_stop = -1
	if lane < 0:
		return
	if stop >= 0:
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
		_draft_pin("stop" + str(index), int(draft_stops[index].lane), Color("#7ee4c8"))
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
			var start = _zone_position(int(edge[0]))
			var destination = _zone_position(int(edge[1]))
			if start != null and destination != null and not start.is_equal_approx(destination):
				draft_marks[key] = _mark_curve(PackedVector3Array([start, destination]), color, 0.23)

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

func _draft_pin(key: String, lane: int, color: Color) -> void:
	var mark := _ground_mark(color)
	var mesh := CylinderMesh.new()
	mesh.top_radius = 0.44
	mesh.bottom_radius = 0.6
	mesh.height = 1.45
	mark.mesh = mesh
	mark.position = city.point_on_lane(lane, 0.9) + Vector3.UP * 0.82
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
	status.text = "Linie in Betrieb."

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
