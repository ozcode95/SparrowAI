// Task types matching the Rust backend
export type ActionType =
  | { type: "ShowNotification"; title: string; message: string }
  | {
      type: "RunMcpFunction";
      server_name: string;
      tool_name: string;
      arguments: any;
    };

export type TriggerTime =
  | { type: "DateTime"; datetime: string }
  | { type: "Daily"; time: string }
  | { type: "Weekly"; day_of_week: number; time: string }
  | { type: "Monthly"; day_of_month: number; time: string };

export type TimeUnit = "Minutes" | "Hours" | "Days" | "Weeks";

export interface RepeatInterval {
  value: number;
  unit: TimeUnit;
}

export interface Task {
  id: string;
  name: string;
  enabled: boolean;
  action_type: ActionType;
  action_params: any;
  trigger_time: TriggerTime;
  repeat_interval?: RepeatInterval;
  created_at: string;
  last_run?: string;
  next_run?: string;
  run_count: number;
  auto_delete: boolean;
}

export interface TaskExecutionLog {
  task_id: string;
  executed_at: string;
  status: "Success" | "Failed" | "Skipped";
  message?: string;
  error?: string;
}
