extends SceneTree

func _initialize() -> void:
	call_deferred("_check_line")

func _check_line() -> void:
	if not ClassDB.class_exists("SimBridge"):
		GDExtensionManager.load_extension("res://sim_bridge.gdextension")
	var core: Object = ClassDB.instantiate("SimBridge")
	if core == null or not core.has_method("line_draft"):
		_fail("Das gemeinsame Linienwerkzeug fehlt.")
		return
	var stops: Array = [{"lane": 1, "zone": 1}, {"lane": 4, "zone": 5}]
	var unreachable: Array = [{"lane": 1, "zone": 6}, {"lane": 4, "zone": 5}]
	if core.line_draft("bus", unreachable, [[], []], 5, false).get("ok", false):
		_fail("Ein Halt darf keinen Fußzugang über eine fremde Zone vortäuschen.")
		return
	var before: int = core.snapshot().stops.size()
	var preview: Dictionary = core.line_draft("bus", stops, [[], []], 5, false)
	if not preview.get("ok", false) or preview.routes.is_empty():
		_fail("Buslinie über die Brücke muss möglich sein: " + str(preview))
		return
	if core.snapshot().stops.size() != before:
		_fail("Vorschauberechnung hat das Netz verändert.")
		return
	var tram_stops: Array = [{"lane": 11, "zone": 4}, {"lane": 12, "zone": 5}]
	var tram: Dictionary = core.line_draft("tram", tram_stops, [[], []], 5, true)
	if not tram.get("ok", false) or tram.routes != [[11, 12], [12, 13, 11]]:
		_fail("Die Straßenbahn muss auf verbundenen Gleisen fahren: " + str(tram))
		return
	var invalid: Dictionary = core.line_draft("tram", stops, [[], []], 5, true)
	if invalid.get("ok", false):
		_fail("Tram wurde auf einer Straße ohne Gleis übernommen.")
		return
	var committed: Dictionary = core.line_draft("bus", stops, [[], []], 5, true)
	if not committed.get("ok", false) or committed.routes != preview.routes:
		_fail("Übernommener Linienweg weicht von der Vorschau ab: " + str(committed))
		return
	var changed: Dictionary = core.line_edit(int(committed.line), "bus", stops, [[], []], 7, true)
	if not changed.get("ok", false) or changed.line != committed.line:
		_fail("Der Takt muss an derselben Linie geändert werden: " + str(changed))
		return
	var saw_bus := false
	for tick in 120:
		core.step(1)
		for lane in core.snapshot().lanes:
			for vehicle in lane.vehicles:
				saw_bus = saw_bus or vehicle.kind == "bus" and int(vehicle.id) > 999
	if not saw_bus:
		_fail("Die eigene Linie hat kein Fahrzeug eingesetzt.")
		return
	var baseline: Object = ClassDB.instantiate("SimBridge")
	var morning_line: Object = ClassDB.instantiate("SimBridge")
	if not morning_line.line_draft("bus", stops, [[], []], 1, true).get("ok", false):
		_fail("Die Linie für den Vergleich muss starten.")
		return
	for model in [baseline, morning_line]:
		model.next_morning()
		model.step(1)
	if _cars_for_bridge(morning_line) >= _cars_for_bridge(baseline):
		_fail("Die neue Linie muss im nächsten Morgenverkehr Autofahrten ersetzen.")
		return
	var waiting := 0
	for stop in morning_line.snapshot().stops:
		waiting += int(stop.waiting)
	if waiting == 0:
		_fail("Auf die neue Linie müssen Fahrgäste warten.")
		return
	print("Vorschau, Gleisbindung, Linienbetrieb und Wirkung im Morgenverkehr geprüft.")
	quit()

func _cars_for_bridge(core: Object) -> int:
	var count := 0
	for flow in core.private_flows(3):
		count += int(flow.count)
	return count

func _fail(message: String) -> void:
	push_error(message)
	quit(1)
