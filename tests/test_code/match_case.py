def func(command):
    match command:
        case "start":
            print("starting")
        case "stop":
            print("stopping")
        case "restart":
            print("restarting")
        case _:
            print("unknown command")
