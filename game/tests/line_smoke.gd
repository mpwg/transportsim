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
	var before: int = core.snapshot().stops.size()
	var preview: Dictionary = core.line_draft("bus", stops, [[], []], 5, false)
	if not preview.get("ok", false) or preview.routes.is_empty():
		_fail("Buslinie über die Brücke muss möglich sein: " + str(preview))
		return
	if core.snapshot().stops.size() != before:
		_fail("Vorschauberechnung hat das Netz verändert.")
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
	print("Vorschau, Gleisbindung und tatsächlicher Linienbetrieb geprüft.")
	quit()

func _fail(message: String) -> void:
	push_error(message)
	quit(1)
