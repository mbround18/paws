from python_fixture import main


def test_main_runs_without_error(capsys):
    main()
    captured = capsys.readouterr()
    assert "Hello from python-fixture!" in captured.out
