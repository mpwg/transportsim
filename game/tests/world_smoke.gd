extends SceneTree

func _initialize() -> void:
	call_deferred("_check_world")

func _check_world() -> void:
	if not ClassDB.class_exists("SimBridge"):
		GDExtensionManager.load_extension("res://sim_bridge.gdextension")
	if not ClassDB.class_exists("SimBridge"):
		_fail("Die native Simulationsbrücke fehlt.")
		return
	var core: Object = ClassDB.instantiate("SimBridge")
	var world: Dictionary = core.world_data()
	if world.get("layout", {}).get("tiles", []).is_empty():
		_fail("Das Szenario muss Gelände liefern.")
		return
	var before: Dictionary = core.snapshot()
	if before.get("stops", []).is_empty() or before.get("lanes", []).is_empty():
		_fail("Der Kern muss Halte und Verkehr liefern.")
		return
	core.step(22)
	var after: Dictionary = core.snapshot()
	if int(after.tick) != int(before.tick) + 22:
		_fail("Die Kernuhr ist nicht um 22 Schritte vorgerückt.")
		return
	print("Brückenstadt aus dem Rust-Kern geladen und weitergeschaltet.")
	quit()

func _fail(message: String) -> void:
	push_error(message)
	quit(1)
