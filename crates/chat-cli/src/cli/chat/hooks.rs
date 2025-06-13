#[cfg(test)]
mod tests {
    use std::io::Stdout;
    use std::time::Duration;

    use tokio::time::sleep;

    use super::*;

    #[test]
    fn test_hook_creation() {
        let command = "echo 'hello'";
        let hook = Hook::new_inline_hook(HookTrigger::PerPrompt, command.to_string());

        assert_eq!(hook.r#type, HookType::Inline);
        assert!(!hook.disabled);
        assert_eq!(hook.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(hook.max_output_size, DEFAULT_MAX_OUTPUT_SIZE);
        assert_eq!(hook.cache_ttl_seconds, DEFAULT_CACHE_TTL_SECONDS);
        assert_eq!(hook.command, Some(command.to_string()));
        assert_eq!(hook.trigger, HookTrigger::PerPrompt);
        assert!(!hook.is_global);
    }

    #[tokio::test]
    async fn test_hook_executor_cached_conversation_start() {
        let mut executor = HookExecutor::new();
        let mut hook1 = Hook::new_inline_hook(HookTrigger::ConversationStart, "echo 'test1'".to_string());
        hook1.is_global = true;

        let mut hook2 = Hook::new_inline_hook(HookTrigger::ConversationStart, "echo 'test2'".to_string());
        hook2.is_global = false;

        // First execution should run the command
        let mut output = Vec::new();
        let results = executor.run_hooks(vec![&hook1, &hook2], Some(&mut output)).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].1.contains("test1"));
        assert!(results[1].1.contains("test2"));
        assert!(!output.is_empty());

        // Second execution should use cache
        let mut output = Vec::new();
        let results = executor.run_hooks(vec![&hook1, &hook2], Some(&mut output)).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].1.contains("test1"));
        assert!(results[1].1.contains("test2"));
        assert!(output.is_empty()); // Should not have run the hook, so no output.
    }

    #[tokio::test]
    async fn test_hook_executor_cached_per_prompt() {
        let mut executor = HookExecutor::new();
        let mut hook1 = Hook::new_inline_hook(HookTrigger::PerPrompt, "echo 'test1'".to_string());
        hook1.is_global = true;
        hook1.cache_ttl_seconds = 60;

        let mut hook2 = Hook::new_inline_hook(HookTrigger::PerPrompt, "echo 'test2'".to_string());
        hook2.is_global = false;
        hook2.cache_ttl_seconds = 60;

        // First execution should run the command
        let mut output = Vec::new();
        let results = executor.run_hooks(vec![&hook1, &hook2], Some(&mut output)).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].1.contains("test1"));
        assert!(results[1].1.contains("test2"));
        assert!(!output.is_empty());

        // Second execution should use cache
        let mut output = Vec::new();
        let results = executor.run_hooks(vec![&hook1, &hook2], Some(&mut output)).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].1.contains("test1"));
        assert!(results[1].1.contains("test2"));
        assert!(output.is_empty()); // Should not have run the hook, so no output.
    }

    #[tokio::test]
    async fn test_hook_executor_not_cached_per_prompt() {
        let mut executor = HookExecutor::new();
        let mut hook1 = Hook::new_inline_hook(HookTrigger::PerPrompt, "echo 'test1'".to_string());
        hook1.is_global = true;

        let mut hook2 = Hook::new_inline_hook(HookTrigger::PerPrompt, "echo 'test2'".to_string());
        hook2.is_global = false;

        // First execution should run the command
        let mut output = Vec::new();
        let results = executor.run_hooks(vec![&hook1, &hook2], Some(&mut output)).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].1.contains("test1"));
        assert!(results[1].1.contains("test2"));
        assert!(!output.is_empty());

        // Second execution should use cache
        let mut output = Vec::new();
        let results = executor.run_hooks(vec![&hook1, &hook2], Some(&mut output)).await;

        assert_eq!(results.len(), 2);
        assert!(results[0].1.contains("test1"));
        assert!(results[1].1.contains("test2"));
        assert!(!output.is_empty());
    }

    #[tokio::test]
    async fn test_hook_timeout() {
        let mut executor = HookExecutor::new();
        let mut hook = Hook::new_inline_hook(HookTrigger::PerPrompt, "sleep 2".to_string());
        hook.timeout_ms = 100; // Set very short timeout

        let results = executor.run_hooks(vec![&hook], None::<&mut Stdout>).await;

        assert_eq!(results.len(), 0); // Should fail due to timeout
    }

    #[tokio::test]
    async fn test_disabled_hook() {
        let mut executor = HookExecutor::new();
        let mut hook = Hook::new_inline_hook(HookTrigger::PerPrompt, "echo 'test'".to_string());
        hook.disabled = true;

        let results = executor.run_hooks(vec![&hook], None::<&mut Stdout>).await;

        assert_eq!(results.len(), 0); // Disabled hook should not run
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let mut executor = HookExecutor::new();
        let mut hook = Hook::new_inline_hook(HookTrigger::PerPrompt, "echo 'test'".to_string());
        hook.cache_ttl_seconds = 1;

        // First execution
        let results1 = executor.run_hooks(vec![&hook], None::<&mut Stdout>).await;
        assert_eq!(results1.len(), 1);

        // Wait for cache to expire
        sleep(Duration::from_millis(1001)).await;

        // Second execution should run command again
        let results2 = executor.run_hooks(vec![&hook], None::<&mut Stdout>).await;
        assert_eq!(results2.len(), 1);
    }

    #[test]
    fn test_hook_cache_storage() {
        let mut executor: HookExecutor = HookExecutor::new();
        let hook = Hook::new_inline_hook(HookTrigger::PerPrompt, "".to_string());

        let cached_hook = CachedHook {
            output: "test output".to_string(),
            expiry: None,
        };

        executor.insert_cache(&hook, cached_hook.clone());

        assert_eq!(executor.get_cache(&hook), Some("test output".to_string()));
    }

    #[test]
    fn test_hook_cache_storage_expired() {
        let mut executor: HookExecutor = HookExecutor::new();
        let hook = Hook::new_inline_hook(HookTrigger::PerPrompt, "".to_string());

        let cached_hook = CachedHook {
            output: "test output".to_string(),
            expiry: Some(Instant::now()),
        };

        executor.insert_cache(&hook, cached_hook.clone());

        // Item should not return since it is expired
        assert_eq!(executor.get_cache(&hook), None);
    }

    #[tokio::test]
    async fn test_max_output_size() {
        let mut executor = HookExecutor::new();

        // Use different commands based on OS
        #[cfg(unix)]
        let command = "for i in {1..1000}; do echo $i; done";

        #[cfg(windows)]
        let command = "for /L %i in (1,1,1000) do @echo %i";

        let mut hook = Hook::new_inline_hook(HookTrigger::PerPrompt, command.to_string());
        hook.max_output_size = 100;

        let results = executor.run_hooks(vec![&hook], None::<&mut Stdout>).await;

        assert!(results[0].1.len() <= hook.max_output_size + " ... truncated".len());
    }

    #[tokio::test]
    async fn test_os_specific_command_execution() {
        let mut executor = HookExecutor::new();

        // Create a simple command that outputs the shell name
        #[cfg(unix)]
        let command = "echo $SHELL";

        #[cfg(windows)]
        let command = "echo %ComSpec%";

        let hook = Hook::new_inline_hook(HookTrigger::PerPrompt, command.to_string());

        let results = executor.run_hooks(vec![&hook], None::<&mut Stdout>).await;

        assert_eq!(results.len(), 1, "Command execution should succeed");

        // Verify output contains expected shell information
        #[cfg(unix)]
        assert!(results[0].1.contains("/"), "Unix shell path should contain '/'");

        #[cfg(windows)]
        assert!(
            results[0].1.to_lowercase().contains("cmd.exe") || results[0].1.to_lowercase().contains("command.com"),
            "Windows shell path should contain cmd.exe or command.com"
        );
    }
}
