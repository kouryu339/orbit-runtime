#include <corework_runtime/runtime.hpp>

#include <cstdlib>
#include <iostream>

int main(int argc, char** argv)
{
    if (argc < 5) {
        std::cerr << "usage: minimal_runtime_host <agent_runtime_lib> <resources.json> <llm.json> <agent-cluster.json>\n";
        return 2;
    }

    try {
        corework::runtime::RuntimeCreateOptions options;
        options.data_dir = "./data/runtime";

        auto runtime = corework::runtime::Runtime::load(
            argv[1],
            options,
            {
                "runtime.register_resources",
                "runtime.register_llm",
                "runtime.register_agent_cluster",
                "conversation.spawn"
            });
        std::cout << "Runtime version: " << runtime.version() << "\n";
        std::cout << "Capabilities: " << runtime.capabilities_json() << "\n";

        runtime.register_resources_path(argv[2]);
        runtime.register_llm_path(argv[3]);
        runtime.register_agent_cluster_path(argv[4]);
        runtime.start();

        corework::runtime::ConversationSpawnOptions spawn;
        spawn.cluster_id = "product-instance";
        spawn.tool_host_context_json = "{}";
        auto conversation = runtime.spawn_conversation(spawn);
        std::cout << "Conversation: " << conversation.conversation_id << "\n";
        runtime.shutdown();
        runtime.destroy();
        return 0;
    } catch (const corework::runtime::RuntimeError& error) {
        std::cerr << error.what() << "\n";
        if (!error.error_json().empty()) {
            std::cerr << error.error_json() << "\n";
        }
        return 1;
    }
}
