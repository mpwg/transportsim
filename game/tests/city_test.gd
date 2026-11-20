extends SceneTree

var _failed := false


func _initialize() -> void:
	call_deferred("_run")


func _run() -> void:
	var script := load("res://city.gd") as GDScript
	if script == null or not script.can_instantiate():
		_expect(false, "Die Stadtansicht lässt sich laden.")
		quit(1)
		return
	var city: Node3D = script.new()
	root.add_child(city)
	var left := {"x": 0, "y": 0}
	var right := {"x": 1, "y": 0}
	var layout := {
		"tiles": [{"cell": left, "terrain": "land"}, {"cell": right, "terrain": "water"}],
		"streets": [
			{"lane": 6, "cells": [left, right], "bridge": true},
			{"lane": 7, "cells": [right, left], "bridge": true},
		],
	}
	var stops := [{"id": 1, "lane": 6}, {"id": 2, "lane": 6}]
	city.build(layout, [{"id": 6, "allowed": ["car", "bus"]}], stops)
	var forward: Vector3 = city.point_on_lane(6, 0.5)
	var backward: Vector3 = city.point_on_lane(7, 0.5)
	_expect(is_equal_approx(forward.x, 4.0), "Eine Zelle umfasst acht Rastereinheiten.")
	_expect(forward.z != backward.z, "Gegenverkehr fährt auf einer eigenen Spur.")
	_expect(city.lane_at(forward) == 6, "Picking trifft die vorwärts führende Spur.")
	_expect(city.lane_at(backward) == 7, "Picking trifft die Gegenrichtung.")
	_expect(city.lane_at(Vector3(4, 0, 8)) == -1, "Neben der Straße wird keine Spur getroffen.")
	_expect(city.lane_curve(20).is_empty(), "Fehlende Spuren liefern keinen Weg.")
	_expect(city.point_on_lane(6, -1.0) == city.point_on_lane(6, 0.0), "Fortschritt wird begrenzt.")
	_expect(city.get_node("Stops/Stop1").position != city.get_node("Stops/Stop2").position,
		"Unterschiedliche Halte auf derselben Spur sind einzeln sichtbar.")
	_expect(city.has_method("stop_position"), "Die tatsächliche Position eines Halts ist für das Kartenwerkzeug verfügbar.")
	if city.has_method("stop_position"):
		_expect(city.stop_position(1) == city.get_node("Stops/Stop1").position, "Die Kartenmarkierung liegt auf dem Halt.")

	var traffic := {
		"lanes": [
			{"id": 6, "vehicles": [{"id": 70, "kind": "bus", "progress": 0.25}]},
			{"id": 7, "vehicles": [{"id": 79, "kind": "car", "progress": 1.0},
				{"id": 80, "kind": "car", "progress": 1.0}]},
		],
		"stops": [{"id": 1, "lane": 6, "waiting": 3}],
	}
	city.show_traffic(traffic, 1.0)
	var bus := city.get_node_or_null("Vehicles/Vehicle70") as Node3D
	_expect(bus != null, "Fahrzeuge kommen aus dem Simulationssnapshot.")
	if bus != null:
		var from := bus.position
		traffic.lanes[0].vehicles[0].progress = 0.75
		city.show_traffic(traffic, 0.5)
		_expect(bus == city.get_node_or_null("Vehicles/Vehicle70"), "Fahrzeugknoten bleiben stabil.")
		_expect(is_equal_approx(bus.position.x, forward.x), "Fortschritt nach neuer Stichprobe wird interpoliert.")
		_expect(bus.position.x > from.x, "Der Bus bewegt sich in Spurrichtung.")
		traffic.lanes[0].vehicles.clear()
		city.show_traffic(traffic, 1.0)
		_expect(city.get_node_or_null("Vehicles/Vehicle70") == null, "Ausgefahrene Fahrzeuge verschwinden.")
	var first := city.get_node("Vehicles/Vehicle79") as Node3D
	var second := city.get_node("Vehicles/Vehicle80") as Node3D
	_expect(first.position.x < second.position.x, "Fahrzeuge am Spurendpunkt stehen hintereinander.")
	var crowd := city.get_node("Stops/Stop1/Crowd")
	_expect(crowd.get_child_count() == 3, "Die Wartenden entsprechen dem Snapshot.")
	traffic.stops.clear()
	traffic.lanes[0].cars = 4
	traffic.lanes[0].ready_cars = 3
	traffic.tick = 90
	city.show_traffic(traffic, 1.0)
	var queue_head := city.get_node_or_null("Vehicles/QueueCar6_0") as Node3D
	var queue_tail := city.get_node_or_null("Vehicles/QueueCar6_2") as Node3D
	_expect(queue_head != null and queue_tail != null and queue_head.position.x > queue_tail.position.x,
		"Aggregierte Autos stehen hintereinander am Spurendpunkt.")
	_expect(city.get_node_or_null("Vehicles/MovingCar6_0") != null,
		"Autos in Fahrt bleiben neben dem Stau sichtbar.")
	_expect(crowd.get_child_count() == 0, "Nach dem Einstieg leert sich die Haltestelle.")

	traffic.lanes[0].cars = 99
	traffic.lanes[0].ready_cars = 99
	traffic.lanes[0].vehicles = [{"id": 70, "kind": "bus", "progress": 0.25}]
	traffic.tick = 91
	traffic.signals = [{"lane": 6, "green": false}]
	city.show_traffic(traffic, 1.0)
	_expect(city.get_node_or_null("Vehicles/Vehicle70") != null,
		"Der Linienbus bleibt auch in einer langen Autoschlange sichtbar.")
	var red := city.get_node_or_null("Signals/Signal6/Red") as MeshInstance3D
	_expect(red != null, "Eine gemeldete Ampel steht an der geregelten Spur.")
	if red != null:
		var red_color: Color = red.material_override.albedo_color
		traffic.tick = 92
		traffic.signals[0].green = true
		city.show_traffic(traffic, 1.0)
		_expect(red_color.r > red.material_override.albedo_color.r, "Der Ampelwechsel wird dargestellt.")

	city.build({}, [], [])
	_expect(city.get_node("Vehicles").get_child_count() == 0, "Ein neues Layout verwirft den sichtbaren Verkehr.")
	_expect(city.lane_at(forward) == -1, "Ein neues Layout verwirft die alten Wege.")
	city.free()
	if not _failed:
		print("Modellstadt geprüft: Wege, Halte, Fahrzeuge, Wartende und Layoutwechsel")
	quit(1 if _failed else 0)


func _expect(condition: bool, message: String) -> void:
	if not condition:
		push_error(message)
		_failed = true
