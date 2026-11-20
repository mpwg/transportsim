extends SceneTree

func _initialize() -> void:
	call_deferred("_check_onboarding")

func _check_onboarding() -> void:
	var scene := load("res://main.tscn") as PackedScene
	if scene == null:
		_fail("Die Spielszene konnte nicht geladen werden.")
		return
	var game := scene.instantiate()
	root.add_child(game)
	await process_frame

	var guide_title := game.get("guide_title") as Label
	var guide_text := game.get("guide_text") as Label
	if guide_title == null or guide_text == null:
		_fail("Beim Spielstart fehlen die sichtbaren Einstiegshinweise.")
		return
	if not guide_title.is_visible_in_tree() or not guide_text.is_visible_in_tree():
		_fail("Die Einstiegshinweise müssen beim Spielstart sichtbar sein.")
		return
	if not _check_title(guide_title, "Erkunden"):
		return
	if not guide_text.text.contains("Mausrad"):
		_fail("Der Einstiegshinweis muss das Zoomen per Mausrad erklären.")
		return

	await process_frame
	if game.guide_card.size.y > 210:
		_fail("Das Hinweisfeld darf die Stadtansicht nicht verdecken.")
		return
	var road_point: Vector3 = game.city.point_on_lane(3, 0.55)
	game._inspect(game.camera.unproject_position(road_point))
	if game.selected_lane < 0 or not is_instance_valid(game.selection_mark) or not game.guide_action.is_visible_in_tree():
		_fail("Die Straßenauswahl muss auf der Karte und durch eine Aktion erkennbar sein.")
		return
	game.guide_action.pressed.emit()
	if not game.shows_journeys:
		_fail("Der Straßenhinweis soll direkt zu den Autowegen führen.")
		return
	game.line_button.set_pressed(true)
	if not _check_title(guide_title, "Erster Halt"):
		return
	if not game.stop_markers.has(1):
		_fail("Mögliche Halte müssen auf der Karte markiert sein.")
		return
	game._add_stop(1)
	if not _check_title(guide_title, "Zweiter Halt"):
		return
	game._add_stop(4)
	if not _check_title(guide_title, "Linie prüfen"):
		return
	if not game.launch_button.is_visible_in_tree() or game.launch_button.disabled:
		_fail("Eine gültige Linie muss sich über die sichtbare Schaltfläche starten lassen.")
		return

	game.launch_button.pressed.emit()
	if game.drafting or not game.draft_stops.is_empty():
		_fail("Der Linienstart muss den Entwurf schließen.")
		return
	if not _check_title(guide_title, "Linie fährt"):
		return
	print("Sichtbare Einstiegshinweise vom Erkunden bis zum Linienstart geprüft.")
	quit()

func _check_title(label: Label, expected: String) -> bool:
	if not label.is_visible_in_tree() or not label.text.contains(expected):
		_fail("Sichtbarer Titel mit „%s“ erwartet, tatsächlicher Titel „%s“." % [expected, label.text])
		return false
	return true

func _fail(message: String) -> void:
	push_error(message)
	quit(1)
