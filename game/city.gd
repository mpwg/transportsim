extends Node3D

const CELL_SIZE := 8.0
const WATER_LEVEL := -1.2
const ROAD_LEVEL := 0.0
const LAND_LEVEL := -0.08
const PICK_RADIUS := 2.75
const CROWD_LIMIT := 24
const WALKER_GROUPS_PER_EDGE := 6
const WALKERS_PER_GROUP := 3
const BOARDING_LIMIT := 5
const ASPHALT := Color("#575a57")
const TRACK_BED := Color("#77786b")
const PAVING := Color("#cec2a8")
const RAIL := Color("#c7bfb0")
const METAL := Color("#475d56")
const BANK := Color("#a79b7d")
const WINDOW := Color("#536b6e")
const WINDOW_TRIM := Color("#e0d3b6")
const BRICK := Color("#a15b44")
const CAR_COLORS := [
	Color("#c76a4c"), Color("#839ca3"), Color("#e8ce8d"),
	Color("#a4a78b"), Color("#d4c9b5"), Color("#677472"), Color("#3e626f"),
]
const COATS := [
	Color("#c76a4c"), Color("#ddb469"), Color("#43808c"),
	Color("#946d77"), Color("#7a805e"), Color("#e0c7a7"),
]

var _curves: Dictionary = {}
var _curve_lengths: Dictionary = {}
var _travel_ticks: Dictionary = {}
var _terrain: Dictionary = {}
var _road_cells: Dictionary = {}
var _tram_lanes: Dictionary = {}
var _stop_nodes: Dictionary = {}
var _signal_nodes: Dictionary = {}
var _signal_materials: Dictionary = {}
var _vehicle_states: Dictionary = {}
var _walking_edges: Dictionary = {}
var _boarders: Array[Dictionary] = []
var _next_walk_group := 0
var _materials: Dictionary = {}
var _batches: Dictionary = {}
var _primitives: Dictionary = {}
var _last_snapshot: Dictionary = {}
var _last_tick: Variant
var _last_blend := 1.0
var _static_root: Node3D
var _vehicle_root: Node3D
var _stop_root: Node3D
var _signal_root: Node3D
var _walking_root: Node3D
var _boarding_root: Node3D


func build(layout: Dictionary, lanes: Array, stops: Array) -> void:
	for child in get_children():
		remove_child(child)
		child.queue_free()
	_curves.clear()
	_curve_lengths.clear()
	_travel_ticks.clear()
	_terrain.clear()
	_road_cells.clear()
	_tram_lanes.clear()
	_stop_nodes.clear()
	_signal_nodes.clear()
	_vehicle_states.clear()
	_walking_edges.clear()
	_boarders.clear()
	_next_walk_group = 0
	_last_snapshot = {}
	_last_tick = null
	_last_blend = 1.0
	_static_root = _group(self, "Model")
	_vehicle_root = _group(self, "Vehicles")
	_stop_root = _group(self, "Stops")
	_signal_root = _group(self, "Signals")
	_walking_root = _group(self, "Walkers")
	_boarding_root = _group(self, "Boarding")

	for lane in lanes:
		var allowed: Array = lane.get("allowed", [])
		_tram_lanes[int(lane.id)] = allowed.has("tram") and not allowed.has("car")
		_travel_ticks[int(lane.id)] = maxf(1.0, float(lane.get("travel_ticks", 5)))
	for tile in layout.get("tiles", []):
		_terrain[_cell(tile.cell)] = tile.terrain
	for street in layout.get("streets", []):
		_record_street(street)
	_join_road_cells()
	_draw_terrain()
	_draw_streets()
	_draw_buildings(layout.get("buildings", []))
	_draw_stops(stops)
	_flush_batches()


func lane_curve(lane: int) -> PackedVector3Array:
	return _curves.get(lane, PackedVector3Array()).duplicate()


func point_on_lane(lane: int, fraction: float) -> Vector3:
	if not _curves.has(lane):
		return Vector3.ZERO
	var points: PackedVector3Array = _curves[lane]
	var lengths: PackedFloat32Array = _curve_lengths[lane]
	var distance := clampf(fraction, 0.0, 1.0) * lengths[-1]
	for index in range(1, points.size()):
		if distance <= lengths[index]:
			var weight := inverse_lerp(lengths[index - 1], lengths[index], distance)
			return points[index - 1].lerp(points[index], weight)
	return points[-1]


func lane_at(point: Vector3) -> int:
	var target := Vector2(point.x, point.z)
	var best_distance := PICK_RADIUS * PICK_RADIUS
	var best_lane := -1
	for lane in _curves:
		var points: PackedVector3Array = _curves[lane]
		for index in range(1, points.size()):
			var from := Vector2(points[index - 1].x, points[index - 1].z)
			var to := Vector2(points[index].x, points[index].z)
			var closest := Geometry2D.get_closest_point_to_segment(target, from, to)
			var distance := target.distance_squared_to(closest)
			if distance < best_distance or (distance == best_distance and lane < best_lane):
				best_distance = distance
				best_lane = lane
	return best_lane


func stop_position(stop_id: int) -> Vector3:
	if not _stop_nodes.has(stop_id):
		return Vector3.ZERO
	var stop: Node3D = _stop_nodes[stop_id].node
	return stop.position


func show_traffic(snapshot: Dictionary, blend: float) -> void:
	if _vehicle_root == null:
		return
	blend = clampf(blend, 0.0, 1.0)
	var tick: Variant = snapshot.get("tick")
	var changed := snapshot != _last_snapshot if tick == null else not is_same(snapshot, _last_snapshot)
	var new_snapshot: bool = changed or tick != _last_tick or blend < _last_blend
	if new_snapshot:
		_sync_vehicles(snapshot.get("lanes", []), float(snapshot.get("tick", 0)))
		_sync_walkers(snapshot.get("walkers", []))
		_clear_boarders()
		_sync_crowds(snapshot.get("stops", []))
		_sync_signals(snapshot.get("signals", []))
		_last_snapshot = snapshot.duplicate(true) if tick == null else snapshot
		_last_tick = tick
	_last_blend = blend
	for state in _vehicle_states.values():
		var node: Node3D = state.node
		node.position = state.from.lerp(state.to, blend)
		node.rotation.y = lerp_angle(state.from_heading, state.to_heading, blend)
	_animate_walkers(blend)
	_animate_boarders(blend)


func _cell(spec: Dictionary) -> Vector2i:
	return Vector2i(int(spec.x), int(spec.y))


func _world(cell: Vector2i) -> Vector3:
	return Vector3(cell.x * CELL_SIZE, ROAD_LEVEL, cell.y * CELL_SIZE)


func _record_street(street: Dictionary) -> void:
	var lane := int(street.lane)
	var cells: Array[Vector2i] = []
	for spec in street.cells:
		cells.push_back(_cell(spec))
	if cells.is_empty():
		return

	var tram: bool = _tram_lanes.get(lane, false)
	var width := float(street.get("road_width_mm", 3500)) / 1000.0
	var first_direction := Vector2i.RIGHT
	if cells.size() > 1:
		first_direction = cells[1] - cells[0]
	elif _terrain.has(cells.front() + Vector2i.DOWN):
		first_direction = Vector2i.DOWN
	var directions: Array[Vector2i] = []
	for index in range(cells.size()):
		var cell := cells[index]
		var incoming := first_direction if index == 0 else cell - cells[index - 1]
		var outgoing := incoming if index == cells.size() - 1 else cells[index + 1] - cell
		directions.push_back(outgoing)
		if not _road_cells.has(cell):
			_road_cells[cell] = {"width": 0.0, "tram": tram, "bridge": false, "edges": {}}
		var patch: Dictionary = _road_cells[cell]
		patch.width = maxf(patch.width, clampf(1.55 * width, 4.2, 6.3))
		patch.tram = patch.tram and tram
		patch.bridge = patch.bridge or (street.get("bridge", false) and _terrain.get(cell) == "water")
		patch.edges[-incoming] = true
		patch.edges[outgoing] = true

	var offset := 0.89 if tram else 1.12
	var points := PackedVector3Array()
	var first := Vector3(first_direction.x, 0, first_direction.y)
	points.append(_world(cells.front()) - first * (CELL_SIZE * 0.5) + first.cross(Vector3.UP) * offset)
	for index in range(cells.size()):
		var centre := _world(cells[index])
		var incoming := first_direction if index == 0 else directions[index - 1]
		var outgoing := directions[index]
		var in_vector := Vector3(incoming.x, 0, incoming.y)
		var out_vector := Vector3(outgoing.x, 0, outgoing.y)
		var side_in := in_vector.cross(Vector3.UP) * offset
		var side_out := out_vector.cross(Vector3.UP) * offset
		if incoming == outgoing:
			points.append(centre + side_out)
		else:
			# A quadratic corner keeps moving vehicles aligned to the turning roadway.
			var entry := centre - in_vector * 2.25 + side_in
			var exit := centre + out_vector * 2.25 + side_out
			var corner := centre + side_in + side_out
			for step in range(5):
				var fraction := step / 4.0
				points.append(entry.lerp(corner, fraction).lerp(corner.lerp(exit, fraction), fraction))
	var last_direction: Vector2i = directions.back()
	var last := Vector3(last_direction.x, 0, last_direction.y)
	points.append(_world(cells.back()) + last * (CELL_SIZE * 0.5) + last.cross(Vector3.UP) * offset)
	var lengths := PackedFloat32Array([0.0])
	for index in range(1, points.size()):
		lengths.append(lengths[-1] + points[index - 1].distance_to(points[index]))
	_curves[lane] = points
	_curve_lengths[lane] = lengths


func _join_road_cells() -> void:
	for cell: Vector2i in _road_cells:
		for direction in [Vector2i.RIGHT, Vector2i.DOWN]:
			var neighbor := cell + Vector2i(direction)
			if not _road_cells.has(neighbor):
				continue
			var a: Dictionary = _road_cells[cell]
			var b: Dictionary = _road_cells[neighbor]
			if a.tram != b.tram:
				continue
			a.edges[direction] = true
			b.edges[-direction] = true


func _draw_terrain() -> void:
	for cell: Vector2i in _terrain:
		var centre := _world(cell)
		var random := _random_for(cell)
		if _terrain[cell] == "water":
			_box(centre + Vector3(0, WATER_LEVEL - 0.28, 0),
				Vector3(CELL_SIZE, 0.56, CELL_SIZE), Color("#557f86"))
			for index in range(9):
				var line := centre + Vector3(random.randf_range(-3.66, 3.66), WATER_LEVEL + 0.008,
					random.randf_range(-3.15, 3.15))
				var color := Color("#739d9e") if index % 3 == 0 else Color("#659297")
				_box(line, Vector3(random.randf_range(0.028, 0.085), 0.013,
					random.randf_range(0.38, 1.8)), color, 0.0, 0.0, false)
			continue

		var grass := [
			Color("#89986c"), Color("#8e9c72"), Color("#8b996f"), Color("#8d9c6f"),
		]
		_box(centre + Vector3(0, LAND_LEVEL - 0.72, 0),
			Vector3(CELL_SIZE, 1.44, CELL_SIZE), grass[random.randi_range(0, grass.size() - 1)])
		for direction in [Vector2i.RIGHT, Vector2i.LEFT, Vector2i.UP, Vector2i.DOWN]:
			if _terrain.get(cell + direction, "") != "water":
				continue
			var outward := Vector3(direction.x, 0, direction.y)
			var side := outward.cross(Vector3.UP)
			var edge := centre + outward * (CELL_SIZE * 0.5 - 0.18)
			_box(edge + Vector3(0, -0.64, 0), Vector3(CELL_SIZE, 1.2, 0.36), BANK,
				_heading(outward))
			_box(edge + Vector3(0, -0.07, 0), Vector3(CELL_SIZE, 0.19, 0.47), PAVING,
				_heading(outward))
			for index in range(8):
				var seam := edge + side * (index - 3.5) + outward * 0.19
				_box(seam + Vector3(0, -0.58, 0), Vector3(0.035, 1.02, 0.025),
					Color("#8d866d"), _heading(outward))


func _draw_streets() -> void:
	for cell: Vector2i in _road_cells:
		var patch: Dictionary = _road_cells[cell]
		var centre := _world(cell)
		var width: float = patch.width
		var surface := TRACK_BED if patch.tram else ASPHALT
		var rise := 0.54 if patch.bridge else 0.12
		_box(centre + Vector3(0, -rise * 0.5, 0), Vector3(width, rise, width), surface)
		for direction: Vector2i in patch.edges:
			var vector := Vector3(direction.x, 0, direction.y)
			var arm := (CELL_SIZE - width) * 0.5
			var middle := centre + vector * (CELL_SIZE + width) * 0.25
			_box(middle + Vector3(0, -rise * 0.5, 0),
				Vector3(width, rise, arm), surface, _heading(vector))
		if patch.edges.size() > 2:
			_crossing(centre, width, patch.edges)
			continue

		var direction: Vector2i = patch.edges.keys()[0]
		var forward := Vector3(direction.x, 0, direction.y)
		var lateral := forward.cross(Vector3.UP)
		var yaw := _heading(forward)
		var straight: bool = patch.edges.has(-direction)
		if not straight:
			continue
		for side in [-1.0, 1.0]:
			var edge: Vector3 = centre + lateral * side * (width * 0.5 + 0.46)
			_box(edge + Vector3(0, -0.025, 0), Vector3(0.94, 0.21, CELL_SIZE),
				PAVING, yaw)
			_box(edge - lateral * side * 0.44 + Vector3(0, 0.052, 0),
				Vector3(0.12, 0.125, CELL_SIZE), Color("#e0d4bc"), yaw)
		if patch.tram:
			_draw_track(centre, lateral, yaw, CELL_SIZE)
		else:
			for advance in [-2.6, 1.4]:
				_box(centre + forward * advance + Vector3(0, 0.014, 0),
					Vector3(0.085, 0.021, 1.15), Color("#e4d1a3"), yaw)
		if patch.bridge:
			_draw_bridge(centre, forward, lateral, width, yaw)


func _crossing(centre: Vector3, width: float, edges: Dictionary) -> void:
	for direction: Vector2i in edges:
		var forward := Vector3(direction.x, 0, direction.y)
		var lateral := forward.cross(Vector3.UP)
		for index in range(5):
			var stripe := centre + forward * (width * 0.5 + 0.38) + lateral * (index - 2) * 0.54
			_box(stripe + Vector3(0, 0.016, 0), Vector3(0.28, 0.02, 0.62),
				Color("#dfd5bc"), _heading(forward))


func _draw_track(centre: Vector3, lateral: Vector3, yaw: float, length: float) -> void:
	for offset in [-1.23, -0.55, 0.55, 1.23]:
		_box(centre + lateral * offset + Vector3(0, 0.043, 0),
			Vector3(0.065, 0.065, length), RAIL, yaw)
	for index in range(11):
		_box(centre + lateral.cross(Vector3.UP) * (index - 5) * 0.7 + Vector3(0, 0.014, 0),
			Vector3(2.75, 0.024, 0.15), Color("#67665c"), yaw)


func _draw_bridge(centre: Vector3, forward: Vector3, lateral: Vector3, width: float, yaw: float) -> void:
	for side in [-1.0, 1.0]:
		var rim: Vector3 = centre + lateral * side * (width * 0.5 + 0.92)
		_box(rim + Vector3(0, -0.27, 0), Vector3(0.28, 0.65, CELL_SIZE), BANK, yaw)
		_box(rim + forward * 2.8 + Vector3(0, -0.75, 0), Vector3(0.48, 1.24, 0.6), BANK, yaw)
		_box(rim + forward * -2.8 + Vector3(0, -0.75, 0), Vector3(0.48, 1.24, 0.6), BANK, yaw)
		for rail_height in [0.38, 0.86]:
			_box(rim + Vector3(0, rail_height, 0), Vector3(0.095, 0.095, CELL_SIZE),
				METAL, yaw)
		for index in range(8):
			var post: Vector3 = rim + forward * (index - 3.5)
			_box(post + Vector3(0, 0.46, 0), Vector3(0.099, 0.91, 0.099), METAL)
		for end in [-1.0, 1.0]:
			var gate: Vector3 = rim + forward * end * 3.62
			_box(gate + Vector3(0, 0.64, 0), Vector3(0.39, 1.28, 0.39), BANK)
			_box(gate + Vector3(0, 1.32, 0), Vector3(0.49, 0.16, 0.49), PAVING)
			_round("cylinder", gate + Vector3.UP * 1.78, Vector3(0.075, 0.91, 0.075), METAL)
			_box(gate + Vector3.UP * 2.28, Vector3(0.24, 0.22, 0.24), Color("#ead5a4"))
			_box(gate + Vector3.UP * 2.42, Vector3(0.29, 0.08, 0.29), METAL)


func _draw_buildings(buildings: Array) -> void:
	var occupied := {}
	for site in buildings:
		var cell := _cell(site.cell)
		occupied[cell] = true
		var random := _random_for(cell)
		match int(site.id):
			1:
				_house(_world(cell), 4.9, 4.5, 3.2, Color("#dcb985"), Color("#a6644a"), 0.0, true)
			3, 4:
				_workshop(_world(cell), int(site.id) == 4)
			5:
				_house(_world(cell), 5.3, 4.6, 4.25, Color("#d1ae83"), Color("#71685c"), 0.0)
			_:
				_housing(_world(cell), random)

	for cell: Vector2i in _terrain:
		if _terrain[cell] != "land" or occupied.has(cell) or _road_cells.has(cell):
			continue
		var random := _random_for(cell)
		var centre := _world(cell)
		var front := _road_cells.has(cell + Vector2i.DOWN) or _road_cells.has(cell + Vector2i.UP)
		var shore: bool = _terrain.get(cell + Vector2i.LEFT) == "water" or _terrain.get(cell + Vector2i.RIGHT) == "water"
		if front and random.randf() < 0.67 and not shore:
			_housing(centre, random)
			continue
		if front or shore:
			_garden(centre, random, shore)
		else:
			for index in range(random.randi_range(3, 6)):
				var tree := centre + Vector3(random.randf_range(-3.1, 3.1), 0, random.randf_range(-3.1, 3.1))
				_tree(tree, random.randf_range(0.62, 1.13), index % 3)


func _housing(centre: Vector3, random: RandomNumberGenerator) -> void:
	var plaster := [Color("#d5b6a2"), Color("#d8c5a2"), Color("#c2c5aa"), Color("#e5c59d")]
	var roofs := [Color("#a66652"), Color("#79675b"), Color("#995a47"), Color("#676b64")]
	_box(centre + Vector3(0, -0.04, 0), Vector3(6.65, 0.11, 5.25), Color("#bbc099"))
	for index in range(2):
		var offset := Vector3((index - 0.5) * 3.5, 0, random.randf_range(-0.61, 0.36))
		_house(centre + offset, 2.75, random.randf_range(2.8, 3.65), random.randf_range(2.25, 3.35),
			plaster[random.randi_range(0, 3)], roofs[random.randi_range(0, 3)], 0.0)
		_box(centre + Vector3(offset.x, -0.026, -2.87), Vector3(0.56, 0.09, 2.1), PAVING)
	_tree(centre + Vector3(-2.8, 0, 2.65), 0.73, 0)
	_tree(centre + Vector3(2.6, 0, 2.8), 0.85, 1)


func _house(centre: Vector3, width: float, depth: float, height: float, walls: Color,
		roof: Color, yaw: float, tavern: bool = false) -> void:
	_box(centre + Vector3(0, height * 0.5, 0), Vector3(width, height, depth), walls, yaw)
	_box(centre + Vector3(0, 0.16, 0), Vector3(width + 0.12, 0.32, depth + 0.12),
		walls.darkened(0.23), yaw)
	var pitch := 0.46
	var roof_slope := width * 0.5 / cos(pitch)
	var ridge_height := height + tan(pitch) * width * 0.5
	var ridge_size := Vector3(0.105, 0.12, depth + 0.32)
	_box(centre + Vector3(0, ridge_height, 0), ridge_size, roof.lightened(0.09), yaw)
	for side in [-1.0, 1.0]:
		var roof_panel := centre + Vector3(side * width * 0.25, (ridge_height + height) * 0.5, 0)
		_box(roof_panel, Vector3(roof_slope + 0.25, 0.14, depth + 0.34), roof, yaw, -side * pitch)
		for index in range(1, 5):
			var edge := centre + Vector3(side * width * 0.5 * index / 5.0,
				ridge_height - tan(pitch) * width * 0.5 * index / 5.0 + 0.11, 0)
			_box(edge, Vector3(0.044, 0.034, depth + 0.33), roof.lightened(0.08), yaw, 0.0, false)
		var gable := centre + Vector3(0, height + 0.02, side * depth * 0.5)
		_gable(gable, width, ridge_height - height, walls, side > 0.0)
		for floor_index in range(int(floorf(height / 1.12))):
			var sill := (floor_index + 0.66) * 1.13
			for column in range(3 if width > 4.0 else 2):
				var columns := 3 if width > 4.0 else 2
				var position_x := (column + 0.5) * width / columns - width * 0.5
				var location := centre + Vector3(position_x, sill, side * (depth * 0.5 + 0.021))
				_window(location, 0.52, 0.65, yaw)
			var flank := centre + Vector3(side * (width * 0.5 + 0.022), sill, 0.18)
			_window(flank, 0.52, 0.65, yaw + PI * 0.5)
	_box(centre + Vector3(-0.32, 0.53, -depth * 0.5 - 0.025), Vector3(0.52, 1.02, 0.10),
		Color("#765c4c"), yaw)
	var chimney := centre + Vector3(width * 0.22, ridge_height, depth * 0.17)
	_box(chimney, Vector3(0.42, 0.99, 0.54), BRICK)
	_box(chimney + Vector3(0, 0.55, 0), Vector3(0.48, 0.12, 0.61), BANK)
	if tavern:
		var inn_sign := centre + Vector3(width * 0.5 + 0.24, 2.03, 0.71)
		_box(inn_sign + Vector3(0, 0.39, 0), Vector3(0.58, 0.055, 0.07), METAL)
		_box(inn_sign, Vector3(0.53, 0.67, 0.09), METAL)
		_box(inn_sign, Vector3(0.43, 0.58, 0.102), Color("#b87854"))
		_box(inn_sign, Vector3(0.125, 0.31, 0.113), WINDOW_TRIM, 0.0, -0.32)
		var terrace := centre + Vector3(0, 0, -depth * 0.5 - 1.02)
		_box(terrace, Vector3(5.1, 0.15, 1.95), PAVING)
		for stripe in range(11):
			var canopy := terrace + Vector3((stripe - 5) * 0.38, 1.55, 0.21)
			var color := Color("#aa624b") if stripe % 2 == 0 else Color("#efe0b6")
			_box(canopy, Vector3(0.38, 0.074, 1.33), color)
		for position_x in [-1.93, 1.93]:
			_box(terrace + Vector3(position_x, 0.8, -0.38), Vector3(0.08, 1.6, 0.08), METAL)
		for position_x in [-1.23, 1.23]:
			var table := terrace + Vector3(position_x, 0.38, -0.32)
			_box(table, Vector3(0.72, 0.086, 0.55), roof)
			_box(table - Vector3.UP * 0.16, Vector3(0.08, 0.34, 0.08), METAL)
			for direction in [-1.0, 1.0]:
				_box(table + Vector3(0.47 * direction, -0.14, 0), Vector3(0.26, 0.27, 0.29), roof)


func _gable(centre: Vector3, width: float, height: float, walls: Color, reverse: bool) -> void:
	var surface := SurfaceTool.new()
	surface.begin(Mesh.PRIMITIVE_TRIANGLES)
	var normal := Vector3.BACK if reverse else Vector3.FORWARD
	var points := [
		centre + Vector3(-width * 0.5, -0.035, 0),
		centre + Vector3(width * 0.5, -0.035, 0),
		centre + Vector3(0, height, 0),
	]
	var indices := [0, 2, 1] if reverse else [0, 1, 2]
	for index: int in indices:
		surface.set_normal(normal)
		surface.add_vertex(points[index])
	var mesh := MeshInstance3D.new()
	mesh.mesh = surface.commit()
	mesh.material_override = _material(walls)
	_static_root.add_child(mesh)


func _window(centre: Vector3, width: float, height: float, yaw: float) -> void:
	_box(centre, Vector3(width + 0.12, height + 0.12, 0.095), WINDOW_TRIM, yaw)
	_box(centre, Vector3(width, height, 0.109), WINDOW, yaw)
	_box(centre, Vector3(0.038, height, 0.128), WINDOW_TRIM, yaw)
	_box(centre + Vector3.DOWN * (height * 0.57), Vector3(width + 0.16, 0.085, 0.16),
		WINDOW_TRIM, yaw)


func _workshop(centre: Vector3, chimney: bool) -> void:
	var walls := Color("#ad765d") if chimney else Color("#c18b68")
	var roof := Color("#786c62")
	_box(centre + Vector3(0, -0.035, 0), Vector3(7.3, 0.17, 7.4), PAVING)
	_box(centre + Vector3(0, 1.75, 0), Vector3(5.65, 3.5, 5.05), walls)
	for bay in range(3):
		var position_x := (bay - 1) * 1.87
		_box(centre + Vector3(position_x, 3.86, 0), Vector3(2.08, 0.16, 5.39), roof, 0, 0.37)
		_box(centre + Vector3(position_x + 0.86, 3.79, 0), Vector3(0.14, 0.72, 5.04), WINDOW)
		for side in [-1.0, 1.0]:
			var window := centre + Vector3(position_x, 1.78, side * 2.55)
			_window(window, 0.96, 1.15, 0.0)
			_box(window + Vector3(0, 0.65, 0), Vector3(1.11, 0.16, 0.17), walls.darkened(0.3))
	if chimney:
		var pipe := centre + Vector3(2.56, 3.1, 2.3)
		_round("cylinder", pipe, Vector3(0.75, 6.18, 0.75), BRICK)
		for height in [-2.8, 2.76, 3.0]:
			_round("cylinder", pipe + Vector3.UP * height, Vector3(0.86, 0.22, 0.86), BANK)
		_round("cylinder", pipe + Vector3.UP * 3.12, Vector3(0.49, 0.085, 0.49), ASPHALT)


func _garden(centre: Vector3, random: RandomNumberGenerator, shore: bool) -> void:
	var path_color := Color("#bdad8b")
	var yaw := PI * 0.5 if shore else 0.0
	_box(centre + Vector3(0, -0.048, 0), Vector3(0.86, 0.076, CELL_SIZE), path_color, yaw)
	for index in range(5):
		var position_x := (index - 2) * 1.48
		var side := -1.0 if index % 2 == 0 else 1.0
		_tree(centre + Vector3(position_x, 0, side * random.randf_range(1.55, 2.85)),
			random.randf_range(0.72, 1.06), index % 3)
	for side in [-1.0, 1.0]:
		var bench := centre + Vector3(side * 2.64, 0.3, 0.7 * side)
		_box(bench, Vector3(1.08, 0.07, 0.32), Color("#957650"), yaw)
		_box(bench + Vector3(0, 0.18, -0.15), Vector3(1.08, 0.18, 0.045), Color("#957650"), yaw)
		for leg in [-0.41, 0.41]:
			_box(bench + Vector3(leg, -0.13, 0), Vector3(0.075, 0.28, 0.27), METAL, yaw)


func _tree(base: Vector3, size: float, variety: int) -> void:
	var greens := [Color("#7b925b"), Color("#8b9b64"), Color("#677e54")]
	var leaf: Color = greens[variety]
	_round("cylinder", base + Vector3.UP * size * 0.8, Vector3(0.17, 1.7, 0.17) * size,
		Color("#856b4f"))
	_round("sphere", base + Vector3.UP * size * 1.93, Vector3(1.68, 1.91, 1.56) * size, leaf)
	_round("sphere", base + Vector3(0.35, 1.51, -0.21) * size, Vector3(1.23, 1.26, 1.13) * size,
		leaf.darkened(0.06))
	_round("sphere", base + Vector3(-0.28, 2.42, 0.13) * size, Vector3(1.16, 1.22, 1.11) * size,
		leaf.lightened(0.075))


func _draw_stops(stops: Array) -> void:
	var by_lane := {}
	for stop in stops:
		var lane := int(stop.lane)
		if not by_lane.has(lane):
			by_lane[lane] = []
		by_lane[lane].append(stop)
	for lane: int in by_lane:
		var entries: Array = by_lane[lane]
		if not _curves.has(lane):
			continue
		var length: PackedFloat32Array = _curve_lengths[lane]
		for index in range(entries.size()):
			var stop: Dictionary = entries[index]
			var span := minf(0.6, 4.4 * (entries.size() - 1) / length[-1])
			var progress := 0.5 if entries.size() == 1 else 0.5 - span * 0.5 + span * index / (entries.size() - 1)
			var direction := _lane_direction(lane, progress)
			var offset := direction.cross(Vector3.UP)
			var tram: bool = _tram_lanes.get(lane, false)
			var position := point_on_lane(lane, progress) + offset * (2.87 if tram else 2.26)
			var node := _group(_stop_root, "Stop%d" % int(stop.id))
			node.position = position
			node.rotation.y = _heading(direction)
			var accent := Color("#a65343") if tram else Color("#4a8b84")
			_part(node, Vector3(0, 0.021, 0), Vector3(1.44, 0.17, 3.45), PAVING)
			_part(node, Vector3(0.65, 0.105, 0), Vector3(0.08, 0.026, 3.38), Color("#dcbe7f"))
			for distance in [-0.72, 0.72]:
				_part(node, Vector3(-0.56, 0.55, distance), Vector3(0.076, 1.14, 0.076), accent)
			_part(node, Vector3(-0.26, 1.14, 0), Vector3(0.75, 0.09, 1.69), accent)
			_part(node, Vector3(-0.54, 0.37, 0), Vector3(0.29, 0.092, 1.22), Color("#a68060"))
			_part(node, Vector3(-0.42, 0.87, 1.53), Vector3(0.075, 1.75, 0.075), METAL)
			_part(node, Vector3(-0.42, 1.58, 1.53), Vector3(0.13, 0.37, 0.37), accent)
			_part(node, Vector3(-0.42, 1.58, 1.53), Vector3(0.15, 0.16, 0.16), Color("#f4e9ce"))
			_stop_nodes[int(stop.id)] = {"node": node, "crowd": _group(node, "Crowd"), "lane": lane}


func _sync_signals(signals: Array) -> void:
	var present := {}
	for signal_state in signals:
		var lane := int(signal_state.lane)
		if not _curves.has(lane):
			continue
		if not _signal_nodes.has(lane):
			_signal_nodes[lane] = _traffic_signal(lane)
		var signal_visual: Dictionary = _signal_nodes[lane]
		var green := bool(signal_state.green)
		var red_material := _signal_material(Color("#5e3230") if green else Color("#ed5942"), not green)
		var green_material := _signal_material(Color("#8ddd72") if green else Color("#344c3a"), green)
		for lens in signal_visual.red:
			lens.material_override = red_material
		for lens in signal_visual.green:
			lens.material_override = green_material
		present[lane] = true
	for lane in _signal_nodes.keys():
		if not present.has(lane):
			var node: Node3D = _signal_nodes[lane].node
			_signal_root.remove_child(node)
			node.queue_free()
			_signal_nodes.erase(lane)


func _traffic_signal(lane: int) -> Dictionary:
	var direction := _lane_direction(lane, 1.0)
	var outward := direction.cross(Vector3.UP)
	var signal_node := _group(_signal_root, "Signal%d" % lane)
	signal_node.position = point_on_lane(lane, 1.0) - direction * 0.74 + outward * 1.98
	signal_node.rotation.y = _heading(direction)
	var frame := Color("#3c4942")
	_part(signal_node, Vector3.UP * 0.85, Vector3(0.12, 1.76, 0.12), METAL, "cylinder")
	_part(signal_node, Vector3.UP * 1.80, Vector3(0.36, 0.74, 0.26), frame)
	_part(signal_node, Vector3.UP * 2.19, Vector3(0.44, 0.09, 0.33), frame)
	var red: Array[MeshInstance3D] = []
	var green: Array[MeshInstance3D] = []
	for side in [-1.0, 1.0]:
		red.append(_part(signal_node, Vector3(0, 1.99, 0.134 * side),
			Vector3(0.245, 0.245, 0.091), frame, "sphere"))
		green.append(_part(signal_node, Vector3(0, 1.62, 0.134 * side),
			Vector3(0.245, 0.245, 0.091), frame, "sphere"))
	red.front().name = "Red"
	green.front().name = "Green"
	return {"node": signal_node, "red": red, "green": green}


func _signal_material(color: Color, active: bool) -> StandardMaterial3D:
	if not _signal_materials.has(color):
		var material := StandardMaterial3D.new()
		material.albedo_color = color
		if active:
			material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		_signal_materials[color] = material
	return _signal_materials[color]


func _sync_walkers(edges: Array) -> void:
	var present := {}
	for entry in edges:
		var id := int(entry.edge)
		var start: Vector3 = entry.from
		var end: Vector3 = entry.to
		var reported: Array = entry.groups.filter(func(group: Dictionary) -> bool:
			return int(group.people) > 0)
		if reported.is_empty() or start.is_equal_approx(end):
			continue
		reported.sort_custom(func(first: Dictionary, second: Dictionary) -> bool:
			return float(first.progress) < float(second.progress))
		present[id] = true
		if not _walking_edges.has(id):
			_walking_edges[id] = {"node": _group(_walking_root, "Edge%d" % id), "groups": []}

		var edge: Dictionary = _walking_edges[id]
		var edge_node: Node3D = edge.node
		edge.start = start
		var direction := (end - start).normalized()
		var previous: Array = edge.groups
		var current: Array[Dictionary] = []
		var shown := mini(reported.size(), WALKER_GROUPS_PER_EDGE)
		for index in range(shown):
			# Sample across the edge to keep both arrivals and new walkers visible in busy periods.
			var sample := roundi(index * (reported.size() - 1.0) / (shown - 1.0)) if shown > 1 else 0
			var report: Dictionary = reported[sample]
			var progress := clampf(float(report.progress), 0.0, 1.0)
			var continuity := _match_walking_group(previous, progress)
			var position := start.lerp(end, progress)
			var state: Dictionary
			if continuity >= 0:
				state = previous.pop_at(continuity)
			else:
				var node := _group(edge_node, "Group%d" % _next_walk_group)
				_next_walk_group += 1
				node.position = position
				state = {"node": node, "seed": _next_walk_group + id}
			var group: Node3D = state.node
			group.rotation.y = _heading(direction)
			_resize_walking_group(group, mini(int(report.people), WALKERS_PER_GROUP), state.seed)
			state.from = group.position
			state.to = position
			state.progress = progress
			current.append(state)
		for state in previous:
			_remove_visual(state.node)
		edge.groups = current

	for id in _walking_edges.keys():
		if not present.has(id):
			_remove_visual(_walking_edges[id].node)
			_walking_edges.erase(id)


func _match_walking_group(previous: Array, progress: float) -> int:
	var match_index := -1
	var shortest := INF
	for index in range(previous.size()):
		var advance := progress - float(previous[index].progress)
		if advance >= -0.00001 and advance < shortest:
			match_index = index
			shortest = advance
	return match_index


func _resize_walking_group(group: Node3D, count: int, seed: int) -> void:
	while group.get_child_count() > count:
		_remove_visual(group.get_child(group.get_child_count() - 1))
	while group.get_child_count() < count:
		var index := group.get_child_count()
		var person := _person_model(group, index, seed)
		person.position = Vector3(0.25 * sin(float(index) * 1.85 + seed), 0, -0.32 * index)


func _animate_walkers(blend: float) -> void:
	for edge in _walking_edges.values():
		for state in edge.groups:
			var group: Node3D = state.node
			group.position = state.from.lerp(state.to, blend)
			var travelled := group.position.distance_to(edge.start)
			for person in group.get_children():
				_pose_walker(person, travelled + float(person.get_index()) * 0.13)


func _sync_crowds(stops: Array) -> void:
	var waiting_by_stop := {}
	for entry in stops:
		waiting_by_stop[int(entry.id)] = int(entry.get("waiting", 0))
	for id in _stop_nodes:
		var count := maxi(0, int(waiting_by_stop.get(id, 0)))
		var stop: Dictionary = _stop_nodes[id]
		var crowd: Node3D = stop.crowd
		var shown := mini(count, CROWD_LIMIT)
		var departed := int(stop.get("waiting", count)) - count
		stop.waiting = count
		if departed > 0:
			_start_boarding(stop, mini(departed, mini(crowd.get_child_count(), BOARDING_LIMIT)))
		while crowd.get_child_count() > shown:
			_remove_visual(crowd.get_child(crowd.get_child_count() - 1))
		while crowd.get_child_count() < shown:
			var index := crowd.get_child_count()
			var person := _person_model(crowd, index, int(id))
			var row := int(index / 8.0)
			var column := index % 8
			var shift := 0.055 * sin(float(index * 7 + id))
			person.position = Vector3(0.44 - row * 0.32 + shift, 0.10, (column - 3.5) * 0.39 + shift)
			person.rotation.y = sin(float(index * 2 + id)) * 0.65


func _start_boarding(stop: Dictionary, count: int) -> void:
	var shelter: Node3D = stop.node
	var best_distance := 3.35
	var target := Vector3.ZERO
	var found := false
	for state in _vehicle_states.values():
		if state.lane != stop.lane or not state.kind in ["bus", "tram"]:
			continue
		var position: Vector3 = state.to
		if position.distance_to(state.from) > 0.32:
			continue
		var distance := position.distance_to(shelter.position)
		if distance >= best_distance:
			continue
		var direction := Vector3(sin(state.to_heading), 0, cos(state.to_heading))
		var size := _vehicle_size(state.kind)
		target = position + direction * size.z * 0.24 + direction.cross(Vector3.UP) * size.x * 0.53
		target.y += 0.07
		best_distance = distance
		found = true
	if not found:
		return
	var crowd: Node3D = stop.crowd
	for index in range(count):
		var person := crowd.get_child(crowd.get_child_count() - 1) as Node3D
		person.reparent(_boarding_root)
		var start := person.position
		person.rotation.y = _heading((target - start).normalized())
		_boarders.append({"node": person, "from": start, "to": target, "delay": index * 0.045})


func _animate_boarders(blend: float) -> void:
	if blend >= 1.0:
		_clear_boarders()
		return
	for state in _boarders:
		var person: Node3D = state.node
		var progress := inverse_lerp(float(state.delay), 1.0, maxf(blend, state.delay))
		person.position = state.from.lerp(state.to, progress)
		_pose_walker(person, person.position.distance_to(state.from))


func _clear_boarders() -> void:
	for state in _boarders:
		_remove_visual(state.node)
	_boarders.clear()


func _pose_walker(person: Node3D, travelled: float) -> void:
	var stride := sin(travelled * TAU / 0.72) * 0.49
	var left := person.get_node("LeftLeg") as Node3D
	var right := person.get_node("RightLeg") as Node3D
	left.rotation.x = stride
	right.rotation.x = -stride


func _remove_visual(node: Node) -> void:
	node.get_parent().remove_child(node)
	node.queue_free()


func _person_model(parent: Node3D, index: int, seed: int) -> Node3D:
	var person := _group(parent, "Person%d" % index)
	person.scale = Vector3.ONE * (0.91 + 0.18 * absf(cos(float(index * 5 + seed))))
	var coat: Color = COATS[posmod(index + seed, COATS.size())]
	_part(person, Vector3(0, 0.35, 0), Vector3(0.20, 0.29, 0.22), coat)
	_part(person, Vector3(0, 0.56, 0), Vector3(0.15, 0.16, 0.15), Color("#d7ac8b"), "sphere")
	var left := _part(person, Vector3(-0.057, 0.13, 0), Vector3(0.081, 0.26, 0.11), METAL)
	var right := _part(person, Vector3(0.057, 0.13, 0), Vector3(0.081, 0.26, 0.11), METAL)
	left.name = "LeftLeg"
	right.name = "RightLeg"
	return person


func _sync_vehicles(lanes: Array, tick: float) -> void:
	var shown := {}
	for lane_state in lanes:
		var lane := int(lane_state.id)
		if not _curves.has(lane):
			continue
		var distance: PackedFloat32Array = _curve_lengths[lane]
		var head := distance[-1]
		var vehicles := _lane_vehicles(lane_state, head, tick)
		for entry in vehicles:
			var id: Variant = entry.key
			var kind: String = entry.kind
			var size := _vehicle_size(kind)
			# Order is foremost first. Clamp trailing vehicles behind the real queue head.
			var offset := clampf(float(entry.progress), 0.0, 1.0) * distance[-1]
			var centre := minf(offset, head - size.z * 0.5)
			head = centre - size.z * 0.5 - 0.26
			if centre < -size.z * 0.25:
				continue
			var fraction := clampf(centre / distance[-1], 0.0, 1.0)
			var position := point_on_lane(lane, fraction)
			var heading := _heading(_lane_direction(lane, fraction))
			if not _vehicle_states.has(id):
				var vehicle := _vehicle(kind, entry.name, entry.color_index, size)
				vehicle.position = position
				vehicle.rotation.y = heading
				_vehicle_states[id] = {"node": vehicle}
			var state: Dictionary = _vehicle_states[id]
			var node: Node3D = state.node
			if entry.get("moving", false) and float(entry.progress) + 0.45 < state.get("progress", 0.0):
				node.position = position
				node.rotation.y = heading
			state.progress = entry.progress
			state.kind = kind
			state.lane = lane
			state.from = node.position
			state.to = position
			state.from_heading = node.rotation.y
			state.to_heading = heading
			shown[id] = true

	for id in _vehicle_states.keys():
		if shown.has(id):
			continue
		var node: Node3D = _vehicle_states[id].node
		_vehicle_root.remove_child(node)
		node.queue_free()
		_vehicle_states.erase(id)


func _lane_vehicles(lane_state: Dictionary, length: float, tick: float) -> Array[Dictionary]:
	var lane := int(lane_state.id)
	var output: Array[Dictionary] = []
	var car_size := _vehicle_size("car").z
	var spacing := car_size + 0.26
	var fleet: Array = lane_state.get("vehicles", [])
	var fleet_length := 0.0
	for vehicle in fleet:
		fleet_length += _vehicle_size(str(vehicle.kind)).z + 0.26
	var slots := maxi(0, floori((length - fleet_length) / spacing))
	var cars := clampi(int(lane_state.get("cars", 0)), 0, slots)
	var ready := clampi(int(lane_state.get("ready_cars", 0)), 0, cars)
	for index in range(ready):
		output.append({
			"key": "queue:%d:%d" % [lane, index], "name": "QueueCar%d_%d" % [lane, index],
			"color_index": lane + index, "kind": "car", "progress": 1.0, "order": output.size(),
		})
	for vehicle in fleet:
		var id := int(vehicle.id)
		output.append({
			"key": id, "name": "Vehicle%d" % id, "color_index": id, "kind": str(vehicle.kind),
			"progress": float(vehicle.progress), "order": output.size(),
		})
	var moving := cars - ready
	var travel_ticks: float = _travel_ticks.get(lane, 5.0)
	var free_fraction := maxf(0.0, 1.0 - ready * spacing / length)
	for index in range(moving):
		var phase := fposmod(0.17 + tick / travel_ticks + float(index) / moving, 1.0)
		output.append({
			"key": "moving:%d:%d" % [lane, index], "name": "MovingCar%d_%d" % [lane, index],
			"color_index": lane + index * 3, "kind": "car", "progress": phase * free_fraction,
			"order": output.size(), "moving": true,
		})
	output.sort_custom(func(first: Dictionary, second: Dictionary) -> bool:
		if first.progress == second.progress:
			return first.order < second.order
		return first.progress > second.progress)
	return output


func _vehicle(kind: String, node_name: String, color_index: int, size: Vector3) -> Node3D:
	var model := _group(_vehicle_root, node_name)
	var color: Color = CAR_COLORS[posmod(color_index, CAR_COLORS.size())]
	var glass := Color("#536b72")
	var rubber := Color("#414543")
	var is_transit := kind == "bus" or kind == "tram"
	if is_transit:
		color = Color("#438d86") if kind == "bus" else Color("#a54e3f")
	elif kind == "taxi":
		color = Color("#e2b75c")
	_part(model, Vector3(0, size.y * 0.39 + 0.1, 0), Vector3(size.x, size.y * 0.63, size.z), color)
	_part(model, Vector3(0, 0.145, 0), Vector3(size.x * 0.94, 0.16, size.z * 0.96), rubber)
	if is_transit:
		_part(model, Vector3(0, size.y * 0.79, 0), Vector3(size.x * 0.95, size.y * 0.32, size.z * 0.975), glass)
		_part(model, Vector3(0, size.y * 0.96, 0), Vector3(size.x, 0.12, size.z * 0.985), Color("#e0d0af"))
		for side in [-1.0, 1.0]:
			for index in range(1, 6 if kind == "tram" else 5):
				var distance := (index / (6.0 if kind == "tram" else 5.0) - 0.5) * size.z
				_part(model, Vector3(size.x * 0.48 * side, size.y * 0.79, distance),
					Vector3(0.035, size.y * 0.33, 0.065), color)
		if kind == "tram":
			_part(model, Vector3(0, size.y + 0.10, -0.26), Vector3(0.47, 0.29, 0.78), METAL)
	else:
		_part(model, Vector3(0, size.y * 0.75, -0.09),
			Vector3(size.x * 0.82, size.y * 0.36, size.z * 0.55), glass)
		_part(model, Vector3(0, size.y * 0.95, -0.12),
			Vector3(size.x * 0.78, 0.075, size.z * 0.41), color.lightened(0.095))
		if kind == "taxi":
			_part(model, Vector3(0, size.y + 0.065, -0.12), Vector3(0.42, 0.11, 0.16), WINDOW_TRIM)
	for side in [-1.0, 1.0]:
		for end in [-1.0, 1.0]:
			var wheel := _part(model, Vector3(side * size.x * 0.44, 0.15, end * size.z * 0.34),
				Vector3(0.29, 0.13, 0.29), rubber, "cylinder")
			wheel.rotation.z = PI * 0.5
		_part(model, Vector3(side * size.x * 0.32, 0.34, size.z * 0.505),
			Vector3(0.18, 0.12, 0.025), Color("#f9e7af"))
		_part(model, Vector3(side * size.x * 0.32, 0.34, -size.z * 0.505),
			Vector3(0.15, 0.12, 0.025), Color("#b95340"))
	return model


func _lane_direction(lane: int, fraction: float) -> Vector3:
	var ahead := point_on_lane(lane, minf(1.0, fraction + 0.012))
	var behind := point_on_lane(lane, maxf(0.0, fraction - 0.012))
	return (ahead - behind).normalized()


func _vehicle_size(kind: String) -> Vector3:
	match kind:
		"tram":
			return Vector3(1.29, 1.29, 4.35)
		"bus":
			return Vector3(1.34, 1.16, 3.15)
		_:
			return Vector3(0.97, 0.72, 1.73)


func _random_for(cell: Vector2i) -> RandomNumberGenerator:
	var random := RandomNumberGenerator.new()
	random.seed = hash(cell)
	return random


func _group(parent: Node, group_name: String) -> Node3D:
	var node := Node3D.new()
	node.name = group_name
	parent.add_child(node)
	return node


func _heading(direction: Vector3) -> float:
	return atan2(direction.x, direction.z)


func _part(parent: Node, centre: Vector3, size: Vector3, color: Color, primitive := "box") -> MeshInstance3D:
	var node := MeshInstance3D.new()
	node.mesh = _primitive(primitive)
	node.material_override = _material(color)
	node.position = centre
	node.scale = size
	parent.add_child(node)
	return node


func _box(centre: Vector3, size: Vector3, color: Color, yaw := 0.0,
		roll := 0.0, casts_shadow := true) -> void:
	var basis := Basis.from_euler(Vector3(0, yaw, roll)).scaled_local(size)
	_batch("box", color, Transform3D(basis, centre), casts_shadow)


func _round(primitive: String, centre: Vector3, size: Vector3, color: Color) -> void:
	_batch(primitive, color, Transform3D(Basis.from_scale(size), centre))


func _batch(primitive: String, color: Color, pose: Transform3D, casts_shadow := true) -> void:
	var key := "%s%s%d" % [primitive, color.to_html(), int(casts_shadow)]
	if not _batches.has(key):
		_batches[key] = {
			"primitive": primitive, "color": color, "poses": [], "casts_shadow": casts_shadow,
		}
	var poses: Array = _batches[key].poses
	poses.push_back(pose)


func _flush_batches() -> void:
	for batch in _batches.values():
		var mesh := MultiMesh.new()
		mesh.transform_format = MultiMesh.TRANSFORM_3D
		mesh.mesh = _primitive(batch.primitive)
		mesh.instance_count = batch.poses.size()
		for index in range(mesh.instance_count):
			mesh.set_instance_transform(index, batch.poses[index])
		var node := MultiMeshInstance3D.new()
		node.multimesh = mesh
		if not batch.casts_shadow:
			node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		node.material_override = _material(batch.color)
		_static_root.add_child(node)
	_batches.clear()


func _material(color: Color) -> StandardMaterial3D:
	if not _materials.has(color):
		var material := StandardMaterial3D.new()
		material.albedo_color = color
		material.roughness = 0.86
		_materials[color] = material
	return _materials[color]


func _primitive(primitive: String) -> Mesh:
	if not _primitives.has(primitive):
		match primitive:
			"sphere":
				var sphere := SphereMesh.new()
				sphere.radial_segments = 9
				sphere.rings = 5
				sphere.radius = 0.5
				sphere.height = 1.0
				_primitives[primitive] = sphere
			"cylinder":
				var cylinder := CylinderMesh.new()
				cylinder.top_radius = 0.5
				cylinder.bottom_radius = 0.5
				cylinder.radial_segments = 9
				cylinder.height = 1.0
				_primitives[primitive] = cylinder
			_:
				_primitives[primitive] = BoxMesh.new()
	return _primitives[primitive]
